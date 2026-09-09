//! Existing-device enrollment. A sealed review references a frozen local history;
//! only small summaries reach iced. Acceptance re-reads it on background owners.
use super::{control::Control, enrollment::*, paths::Paths, replica::Replica, *};
use crate::{model::Account, store::Store};
use shep_profile_core::{Action, Change, history};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const STORAGE_KEY: &str = "profile_join_v1";
pub(crate) const RECONNECT_KEY: &str = "profile_reconnect_v1";

#[derive(Clone, Debug)]
pub struct Review {
    pub(crate) id: Uuid,
    pub(crate) local: Snapshot,
    pub(crate) selection: Selection,
    pub(crate) revision: u64,
    pub(crate) device: Uuid,
    pub accounts: usize,
    pub settings: usize,
    pub account_preview: Vec<(String, String)>,
}
impl Review {
    pub fn name(&self) -> &str {
        &self.selection.name
    }
    pub fn local(&self) -> &Snapshot {
        &self.local
    }
}

/// Durable shared-to-local identity mapping; credentials remain device-local.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Applied {
    pub review: Uuid,
    pub binding: history::Binding,
    pub history_revision: u64,
    pub accounts: BTreeMap<Uuid, String>,
}

pub(crate) struct Values {
    pub accounts: Vec<Account>,
    pub settings: Vec<Change>,
}

/// Read only current, conflict-free visible fields, not the operation history.
async fn values(replica: &Replica, options: Options, control: &Control) -> anyhow::Result<Values> {
    let mut connections = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut settings = Vec::new();
    let mut after = None;
    loop {
        control.check()?;
        let fields = replica.fields(after).await?;
        if fields.is_empty() {
            break;
        }
        after = fields.last().map(|f| f.target.clone());
        for field in fields {
            anyhow::ensure!(
                !field.conflict,
                "Resolve conflicting profile fields before joining this device."
            );
            let versions = replica.versions(field.target.clone(), None).await?;
            let version = versions
                .first()
                .context("The reviewed profile field is missing.")?;
            // Concurrent tombstones are equivalent; all other fields need one value.
            anyhow::ensure!(
                versions.len() == 1 || field.target.ends_with(":removed"),
                "Review conflicting profile values first."
            );
            let change = replica.value(field.target, version.operation).await?;
            match change.action {
                Action::AccountConnection { account } if options.accounts => {
                    connections.insert(account.id, account);
                }
                Action::AccountName { id, name } if options.accounts => {
                    names.insert(id, name);
                }
                Action::Setting { .. } | Action::SettingRemoved { .. } if options.settings => {
                    settings.push(change)
                }
                Action::ProfileRemoved => {
                    anyhow::bail!("This shared profile was removed. Choose another profile.")
                }
                _ => {}
            }
        }
    }
    let accounts = connections
        .into_iter()
        .map(|(id, connection)| {
            metadata::review_account(
                &connection,
                names
                    .get(&id)
                    .map_or(connection.email.as_str(), String::as_str),
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    // Validate supported settings before offering a review; unsupported optional
    // fields stay in history and are not counted as applied native settings.
    let mut candidate = crate::model::Preferences::default();
    for change in &settings {
        metadata::apply_setting(&mut candidate, change)?;
    }
    settings.retain(|c| match &c.action {
        Action::Setting { key, .. } | Action::SettingRemoved { key } => {
            metadata::SETTINGS.contains(key)
        }
        _ => false,
    });
    Ok(Values { accounts, settings })
}

fn complete(state: &history::State) -> anyhow::Result<()> {
    anyhow::ensure!(
        !state.removed
            && state.operations > 0
            && state.waiting == 0
            && state.ready == 0
            && state.conflicts == 0
            && state.queued == 0,
        "This profile is incomplete, removed or has conflicting changes. Review it on its original device before joining."
    );
    Ok(())
}

pub(crate) async fn prepare(
    store: &Store,
    session: &drive::Session,
    paths: &Paths,
    journal: journal::Journal,
    discovery: &setup::Discovery,
    cursor: &str,
    control: &Control,
) -> anyhow::Result<Review> {
    discovery.validate(session, &journal).await?;
    store
        .check_profile_review(discovery.local().clone())
        .await?;
    let profile = discovery
        .profiles()
        .iter()
        .find(|p| p.cursor() == cursor)
        .context("Choose a profile from the current discovery page.")?;
    anyhow::ensure!(
        !profile.removed
            && profile.waiting == 0
            && profile.conflicts == 0
            && !profile.name_conflict,
        "This shared profile needs review on its original device before joining."
    );
    let selection = Selection {
        binding: history::Binding {
            namespace: session.binding().namespace().into(),
            principal: session.binding().identity().into(),
            profile: profile.profile,
            generation: profile.generation,
        },
        name: profile
            .name
            .clone()
            .context("Give this profile a name on its original device first.")?,
        origin: Origin::Join,
        ready: true,
    };
    selection.validate()?;
    let mut replica = Replica::open(
        paths.history(&selection.binding)?,
        selection.binding.clone(),
        journal,
    )
    .await?;
    let result = async {
        let pulled = replica.pull_controlled(session, control).await?;
        complete(pulled.state())?;
        anyhow::ensure!(
            pulled.remote_records() > 0,
            "This profile is no longer on Drive. Discover profiles again."
        );
        let names = replica.versions("profile:name".into(), None).await?;
        anyhow::ensure!(
            names.len() == 1,
            "This profile name changed. Discover profiles again."
        );
        let name = replica
            .value("profile:name".into(), names[0].operation)
            .await?;
        anyhow::ensure!(
            matches!(name.action,Action::ProfileName{name} if name == selection.name),
            "This profile was renamed during discovery. Discover profiles again."
        );
        let values = values(&replica, discovery.local().enrollment.options, control).await?;
        store
            .check_profile_review(discovery.local().clone())
            .await?;
        control.check()?;
        Ok(Review {
            id: Uuid::new_v4(),
            local: discovery.local().clone(),
            selection,
            revision: pulled.state().revision,
            device: pulled.state().device,
            accounts: values.accounts.len(),
            settings: values.settings.len(),
            account_preview: values
                .accounts
                .into_iter()
                .take(8)
                .map(|a| (a.name, a.email))
                .collect(),
        })
    }
    .await;
    replica
        .close()
        .await
        .context("Could not finish saving the profile review")?;
    result
}

pub(crate) async fn accept(
    store: &Store,
    paths: &Paths,
    review: Review,
    control: &Control,
) -> anyhow::Result<Snapshot> {
    control.check()?;
    // A lost acceptance acknowledgment is safe to retry without applying again,
    // even if preferences have since changed or the device is now offline.
    if let Some(saved) = store.applied_profile_join(review.id).await? {
        return Ok(saved);
    }
    store.check_profile_review(review.local.clone()).await?;
    let replica = Replica::open(
        paths.history(&review.selection.binding)?,
        review.selection.binding.clone(),
        paths.journal().await?,
    )
    .await?;
    let result = async {
        let state = replica.state().await?;
        complete(&state)?;
        anyhow::ensure!(
            state.revision == review.revision && state.device == review.device,
            "The shared history changed. Review this profile again before applying it."
        );
        let values = values(&replica, review.local.enrollment.options, control).await?;
        anyhow::ensure!(
            values.accounts.len() == review.accounts && values.settings.len() == review.settings,
            "The reviewed profile values changed. Review the profile again."
        );
        control.check()?;
        // Once admitted, keep this atomic write owned through its acknowledgment.
        store.accept_profile_join(review, values).await
    }
    .await;
    replica
        .close()
        .await
        .context("Could not finish saving the joined profile")?;
    result
}

pub(crate) type Reconnect = BTreeSet<String>;

#[cfg(test)]
mod tests;
