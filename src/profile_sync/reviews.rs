//! Bounded, native-setting reviews minted by the exclusive history owner. Wire
//! extensions stay in history; the UI receives only scalar values and identities.
use super::{metadata, replica::Replica, state::Pending};
use crate::store::Store;
use anyhow::{Context, ensure};
use serde_json::Value;
use shep_profile_core::{Action, Change, SettingKey, history};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(crate) struct Basis {
    pub binding: history::Binding,
    pub enrollment_revision: u64,
    pub google_revision: u64,
    pub key: SettingKey,
    pub local: Value,
    pub native_revision: u64,
    pub pending: Option<Uuid>,
    pub changed: bool,
}
#[derive(Clone, Debug)]
pub struct Version {
    pub operation: Uuid,
    pub value: Value,
    pub reset: bool,
}
#[derive(Clone, Debug)]
pub struct Review {
    pub(crate) basis: Basis,
    pub(crate) revision: u64,
    versions: Vec<Version>,
}
impl Review {
    pub fn key(&self) -> SettingKey {
        self.basis.key
    }
    pub fn local(&self) -> &Value {
        &self.basis.local
    }
    pub fn versions(&self) -> &[Version] {
        &self.versions
    }
    pub fn label(&self) -> &'static str {
        match self.key() {
            SettingKey::Appearance => "Appearance",
            SettingKey::ReplyDisplay => "Quoted replies",
            SettingKey::ImagePolicy => "External images",
            SettingKey::UnifiedInbox => "Unified inbox",
            SettingKey::CrossAccountMoves => "Moves between accounts",
            SettingKey::GroupConversations => "Conversation grouping",
            SettingKey::DesktopBadges => "Unread badges",
            SettingKey::Tooltips => "Tooltips",
            _ => "Shared preference",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Choice {
    Local,
    Shared(Uuid),
}

fn target(key: SettingKey) -> String {
    history::target(&Action::SettingRemoved { key })
}
async fn versions(replica: &Replica, key: SettingKey) -> anyhow::Result<Vec<history::Version>> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = replica.versions(target(key), after).await?;
        if page.is_empty() {
            break;
        }
        ensure!(
            all.len() + page.len() <= shep_profile_core::MAX_PARENTS,
            "Too many versions of this preference need review. Update Shep before resolving them."
        );
        after = page.last().map(|v| v.operation);
        all.extend(page);
    }
    Ok(all)
}
fn scalar(key: SettingKey, change: &Change) -> anyhow::Result<Value> {
    match &change.action {
        Action::Setting { key: actual, value } if *actual == key => Ok(value.clone()),
        Action::SettingRemoved { key: actual } if *actual == key => {
            metadata::setting_value(key, &crate::model::Preferences::default())
                .context("Unsupported shared preference")
        }
        _ => anyhow::bail!("The shared preference identity changed. Refresh its review."),
    }
}
fn ready(state: &history::State) -> anyhow::Result<()> {
    ensure!(
        state.initialized && !state.removed && state.ready == 0 && state.waiting == 0,
        "Finish checking shared history before reviewing preferences."
    );
    Ok(())
}

pub async fn prepare(store: &Store, replica: &Replica) -> anyhow::Result<Vec<Review>> {
    let state = replica.state().await?;
    ready(&state)?;
    let mut reviews = Vec::new();
    for basis in store
        .profile_setting_review_bases(replica.binding().clone())
        .await?
    {
        let current = versions(replica, basis.key).await?;
        if current.is_empty() {
            continue;
        }
        let mut candidates = Vec::new();
        for version in current {
            let change = replica.value(target(basis.key), version.operation).await?;
            candidates.push(Version {
                operation: version.operation,
                value: scalar(basis.key, &change)?,
                reset: matches!(change.action, Action::SettingRemoved { .. }),
            });
        }
        if candidates.len() == 1
            && basis.pending.is_none()
            && (!basis.changed || basis.local == candidates[0].value)
        {
            continue;
        }
        reviews.push(Review {
            basis,
            revision: state.revision,
            versions: candidates,
        });
    }
    Ok(reviews)
}

/// Reserve before admission, retaining one UUID and the exact conflict versions
/// through a process restart. No network is needed to make the choice durable.
pub async fn accept(
    store: &Store,
    replica: &mut Replica,
    review: Review,
    choice: Choice,
) -> anyhow::Result<()> {
    ensure!(
        replica.binding() == &review.basis.binding,
        "The shared profile changed. Refresh this review."
    );
    let current = replica.state().await?;
    ready(&current)?;
    let actual = versions(replica, review.key()).await?;
    ensure!(
        current.revision == review.revision
            && actual
                .iter()
                .map(|v| v.operation)
                .eq(review.versions.iter().map(|v| v.operation)),
        "Shared preferences changed while this review was open. Refresh it before choosing."
    );
    let operation = match choice {
        Choice::Local => {
            review
                .versions
                .first()
                .context("No shared preference version remains")?
                .operation
        }
        Choice::Shared(operation) => {
            ensure!(
                review.versions.iter().any(|v| v.operation == operation),
                "Choose a version from this review."
            );
            operation
        }
    };
    let mut change = replica.value(target(review.key()), operation).await?;
    scalar(review.key(), &change)?;
    if matches!(choice, Choice::Local) {
        // Preserve the selected current record's opaque fields. Only the native
        // setting itself is replaced by the explicitly reviewed local value.
        change.action = Action::Setting {
            key: review.key(),
            value: review.local().clone(),
        };
    }
    let pending = store.reserve_profile_setting_review(review, change).await?;
    // After reservation, cancellation cannot drop the exact admitted receipt.
    let receipt = replica.admit_local(pending).await?;
    store.acknowledge_profile_change(receipt).await
}

pub(crate) fn pending(review: &Review, change: Change, native_revision: u64) -> Pending {
    Pending {
        operation: Uuid::new_v4(),
        expected_revision: review.revision,
        local: super::state::normalized(change.clone()),
        change,
        native_revision,
        resolutions: if review.versions.len() > 1 {
            review.versions.iter().map(|v| v.operation).collect()
        } else {
            vec![]
        },
    }
}

#[cfg(test)]
impl Review {
    /// UI ordering fixture only. Production reviews are minted by prepare().
    pub(crate) fn fixture(key: SettingKey, value: Value) -> Self {
        Self {
            basis: Basis {
                binding: history::Binding {
                    namespace: "so.shep".into(),
                    principal: "drive:fixture".into(),
                    profile: Uuid::new_v4(),
                    generation: Uuid::new_v4(),
                },
                enrollment_revision: 1,
                google_revision: 1,
                key,
                local: value.clone(),
                native_revision: 1,
                pending: None,
                changed: true,
            },
            revision: 1,
            versions: vec![Version {
                operation: Uuid::new_v4(),
                value,
                reset: false,
            }],
        }
    }
}
