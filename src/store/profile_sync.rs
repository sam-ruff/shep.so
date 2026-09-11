use super::*;
use crate::profile_sync::enrollment::{
    self, Enrollment, Options, STORAGE_KEY, Selection, Snapshot,
};

fn current(c: &Connection) -> anyhow::Result<Enrollment> {
    let value: Enrollment = get(c, STORAGE_KEY)?;
    value.validate()?;
    Ok(value)
}
fn snapshot(c: &Connection) -> anyhow::Result<Snapshot> {
    let prefs: Preferences = get(c, "preferences")?;
    let enrollment = current(c)?;
    let defaults = Preferences::default();
    let accounts: Vec<Account> = get(c, "accounts")?;
    let empty_workspace = accounts.is_empty()
        && get::<u64>(c, "drafts_revision")? == 0
        && !c.query_row("SELECT EXISTS(SELECT 1 FROM messages LIMIT 1)", [], |r| {
            r.get::<_, bool>(0)
        })?
        && (!enrollment.options.settings
            || crate::profile_sync::metadata::SETTINGS.iter().all(|key| {
                crate::profile_sync::metadata::setting_value(*key, &prefs)
                    == crate::profile_sync::metadata::setting_value(*key, &defaults)
            }));
    Ok(Snapshot {
        enrollment,
        preferences_revision: get(c, "preferences_revision")?,
        connections_revision: get(c, "connections_revision")?,
        google_revision: prefs.google_lifecycle.revision,
        google_identity: prefs.google_connection_id.clone(),
        available: enrollment::google_available(&prefs),
        accounts: accounts.len(),
        empty_workspace,
    })
}
fn review_matches(c: &Connection, expected: &Snapshot) -> anyhow::Result<Enrollment> {
    let now = snapshot(c)?;
    anyhow::ensure!(
        now.enrollment == expected.enrollment
            && now.preferences_revision == expected.preferences_revision
            && now.connections_revision == expected.connections_revision
            && now.google_revision == expected.google_revision
            && now.google_identity == expected.google_identity,
        "Your local setup changed. Refresh the profile review before continuing."
    );
    Ok(now.enrollment)
}

mod account_reviews;
mod continuous;
pub(super) mod join;
mod reviews;
pub(super) mod state;
mod vault;

impl Store {
    pub async fn change_profile_sync_options(
        &self,
        changes: enrollment::Changes,
    ) -> anyhow::Result<Snapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value = current(&tx)?;
            let next = changes.apply(value.options);
            next.validate()?;
            if changes.enabled == Some(true) {
                let selected = value
                    .selection
                    .as_ref()
                    .context("Choose a shared profile before enabling sync.")?;
                enrollment::check_google(&get(&tx, "preferences")?, selected)?;
            }
            if next != value.options {
                vault::toggled(&tx, value.options, next)?;
                value.options = next;
                value.advance()?;
                put(&tx, STORAGE_KEY, &value)?;
            }
            let result = snapshot(&tx)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }
    pub async fn check_profile_review(&self, expected: Snapshot) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            review_matches(&tx, &expected)?;
            Ok(())
        })
        .await
    }
    pub async fn profile_enrollment(&self) -> anyhow::Result<Snapshot> {
        self.run(|c| {
            let tx = c.transaction()?;
            snapshot(&tx)
        })
        .await
    }

    /// Local controls use their own revision, independent of a long Drive job.
    /// Disabling always works without network/keychain access. A stale result
    /// cannot turn sync back on or replace a newer category choice.
    pub async fn set_profile_sync_options(
        &self,
        expected: u64,
        options: Options,
    ) -> anyhow::Result<Snapshot> {
        options.validate()?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value = current(&tx)?;
            anyhow::ensure!(
                value.revision == expected,
                "Profile sync choices changed. Refresh and try again."
            );
            if options.enabled {
                let selection = value
                    .selection
                    .as_ref()
                    .context("Choose a shared profile before enabling sync.")?;
                enrollment::check_google(&get(&tx, "preferences")?, selection)?;
            }
            if value.options != options {
                vault::toggled(&tx, value.options, options)?;
                value.options = options;
                value.advance()?;
                value.validate()?;
                put(&tx, STORAGE_KEY, &value)?;
            }
            let result = snapshot(&tx)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// The provider supplies a verified candidate after complete discovery.
    /// Keep first-device intent durable before generating/uploading local edits.
    pub async fn begin_profile_enrollment(
        &self,
        expected: Snapshot,
        mut selection: Selection,
        options: Options,
    ) -> anyhow::Result<Snapshot> {
        selection.validate()?;
        options.validate()?;
        anyhow::ensure!(
            options.enabled,
            "Enable profile sync when confirming its setup."
        );
        selection.ready = false;
        self.run(move |c| {
            let tx=c.transaction()?;
            let mut value=review_matches(&tx,&expected)?;
            let prefs:Preferences=get(&tx,"preferences")?;
            enrollment::check_google(&prefs,&selection)?;
            if let Some(existing)=&value.selection {
                anyhow::ensure!(existing.binding == selection.binding && existing.origin == selection.origin,
                    "This workspace already has a shared profile. Use another local workspace to keep both profiles separate.");
                anyhow::ensure!(!existing.ready,"This profile is already set up. Use its sync controls.");
                anyhow::ensure!(existing == &selection && value.options == options,
                    "This setup is already pending with different choices. Resume its saved setup.");
                return snapshot(&tx);
            }
            if selection.origin == enrollment::Origin::Create {
                let seed=enrollment::Seed::create(&selection,options,&get::<Vec<Account>>(&tx,"accounts")?,&prefs)?;
                put(&tx,enrollment::SEED_KEY,&seed)?;
            }
            value.selection=Some(selection);
            value.options=options;
            value.last_success=None;
            value.advance()?;
            value.validate()?;
            put(&tx,STORAGE_KEY,&value)?;
            let result=snapshot(&tx)?;
            tx.commit()?;
            Ok(result)
        }).await
    }

    pub async fn profile_seed(&self, expected: Snapshot) -> anyhow::Result<enrollment::Seed> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let value = review_matches(&tx, &expected)?;
            let selection = value
                .selection
                .as_ref()
                .context("Choose a shared profile first.")?;
            let seed: Option<enrollment::Seed> = get(&tx, enrollment::SEED_KEY)?;
            let seed = seed.context(
                "The saved profile setup is missing. Keep the workspace and review setup recovery.",
            )?;
            seed.validate(selection)?;
            Ok(seed)
        })
        .await
    }

    /// A saved pre-publication review may adopt the barrier only before any
    /// history admission. Keep its metadata and operation IDs exactly intact.
    pub(crate) async fn prepare_profile_seed(
        &self,
        expected: Snapshot,
        history_empty: bool,
    ) -> anyhow::Result<enrollment::Seed> {
        self.run(move |c| {
            let tx=c.transaction()?;
            let value=review_matches(&tx,&expected)?;
            let selection=value.selection.as_ref().context("Choose a shared profile first.")?;
            enrollment::check_google(&get(&tx,"preferences")?,selection)?;
            anyhow::ensure!(value.options.enabled,"Profile sync was turned off.");
            let mut seed:enrollment::Seed=get::<Option<enrollment::Seed>>(&tx,enrollment::SEED_KEY)?.context("The saved profile setup is missing.")?;
            if seed.initialization.is_none() {
                anyhow::ensure!(history_empty && seed.chunks.iter().all(|c|c.expected_revision.is_none()),
                    "This older profile has already started publication and needs recovery. Its original records and accounts have been kept; no replacement was uploaded.");
                seed.initialization=Some(enrollment::Initialization::new());
                seed.validate(selection)?;
                put(&tx,enrollment::SEED_KEY,&seed)?;
            } else {seed.validate(selection)?;}
            tx.commit()?;
            Ok(seed)
        }).await
    }

    /// Persist the exact shared-core edit request before submitting it. A retry
    /// must keep the first expected revision as well as its operation ID/bytes.
    pub async fn checkpoint_profile_seed(
        &self,
        expected: Snapshot,
        operation: uuid::Uuid,
        revision: u64,
    ) -> anyhow::Result<enrollment::SeedChunk> {
        anyhow::ensure!(
            revision <= i64::MAX as u64,
            "The setup history revision is invalid."
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let value = review_matches(&tx, &expected)?;
            anyhow::ensure!(value.options.enabled, "Profile sync was turned off.");
            let selection = value
                .selection
                .as_ref()
                .context("Choose a shared profile first.")?;
            enrollment::check_google(&get(&tx, "preferences")?, selection)?;
            let mut seed: enrollment::Seed =
                get::<Option<enrollment::Seed>>(&tx, enrollment::SEED_KEY)?
                    .context("The saved profile setup is missing.")?;
            seed.validate(selection)?;
            let chunk = seed
                .operation_mut(operation)
                .context("This setup operation is no longer pending.")?;
            if chunk.expected_revision.is_none() {
                chunk.expected_revision = Some(revision);
            }
            let result = chunk.clone();
            put(&tx, enrollment::SEED_KEY, &seed)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Call only after the provider/history/application acknowledgments. Even
    /// then a disabled/replaced setup must retain its user's newer choice.
    pub async fn profile_sync_succeeded(
        &self,
        expected: Snapshot,
        when: i64,
    ) -> anyhow::Result<Snapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value = review_matches(&tx, &expected)?;
            anyhow::ensure!(
                value.options.enabled,
                "Profile sync was turned off. The saved remote change was kept."
            );
            let selection = value
                .selection
                .as_mut()
                .context("Choose a shared profile before syncing.")?;
            enrollment::check_google(&get(&tx, "preferences")?, selection)?;
            selection.ready = true;
            value.last_success = Some(when);
            value.advance()?;
            put(&tx, STORAGE_KEY, &value)?;
            let result = snapshot(&tx)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Apply a bounded page of conflict-free settings after a complete history
    /// review. Newer local edits/category choices reject the whole page.
    pub async fn apply_profile_settings(
        &self,
        expected: Snapshot,
        changes: Vec<shep_profile_core::Change>,
    ) -> anyhow::Result<(Snapshot, PreferenceSnapshot)> {
        anyhow::ensure!(
            changes.len() <= shep_profile_core::MAX_CHANGES,
            "Apply shared settings in bounded pages."
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let enrollment = review_matches(&tx, &expected)?;
            anyhow::ensure!(
                enrollment.options.enabled && enrollment.options.settings,
                "Settings sync was turned off. Local preferences were kept."
            );
            let selection = enrollment
                .selection
                .as_ref()
                .context("Choose a shared profile first.")?;
            let mut prefs: Preferences = get(&tx, "preferences")?;
            enrollment::check_google(&prefs, selection)?;
            let mut keys = std::collections::BTreeSet::new();
            let mut changed = false;
            for change in changes {
                use shep_profile_core::Action;
                let key = match &change.action {
                    Action::Setting { key, .. } | Action::SettingRemoved { key } => *key,
                    _ => anyhow::bail!("Review account changes separately from portable settings."),
                };
                anyhow::ensure!(
                    keys.insert(key),
                    "The profile review contains conflicting setting values."
                );
                changed |= crate::profile_sync::metadata::apply_setting(&mut prefs, &change)?;
            }
            prefs.validate()?;
            if changed {
                put(&tx, "preferences", &prefs)?;
            }
            let result = snapshot(&tx)?;
            let preferences = PreferenceSnapshot {
                revision: result.preferences_revision,
                value: prefs,
            };
            tx.commit()?;
            Ok((result, preferences))
        })
        .await
    }
}

/// Run in the same transaction as Google disconnect so pending results are
/// fenced before keychain cleanup. Preserve the profile for an explicit resume.
pub(super) fn pause(c: &Connection) -> anyhow::Result<()> {
    let mut value = match current(c) {
        Ok(value) => value,
        // An unreadable/future enrollment cannot run. The Google lifecycle
        // fence still stops providers; preserve this record for later repair.
        Err(_) => return Ok(()),
    };
    if value.options.enabled {
        value.options.enabled = false;
        value.advance()?;
        put(c, STORAGE_KEY, &value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
