//! Explicit connection choices retain existing native account/mail identities.
//! A changed shared endpoint is added separately; credentials never move with it.
use super::{
    metadata,
    replica::Replica,
    state::{self, Pending},
};
use crate::{model::Account, store::Store};
use anyhow::{Context, ensure};
use shep_profile_core::{Action, Change, history};
use uuid::Uuid;

pub(crate) const PAGE_SIZE: usize = 8;
#[derive(Clone, Debug)]
pub(crate) struct Basis {
    pub binding: history::Binding,
    pub enrollment_revision: u64,
    pub google_revision: u64,
    pub connections_revision: u64,
    pub local: Account,
    pub shared: Uuid,
    pub native_revision: u64,
    pub pending: Option<Uuid>,
}
#[derive(Clone, Debug)]
pub struct Version {
    pub operation: Uuid,
    pub account: Account,
}
#[derive(Clone, Debug)]
pub struct Review {
    pub(crate) basis: Basis,
    pub(crate) revision: u64,
    versions: Vec<Version>,
}
impl Review {
    pub fn local(&self) -> &Account {
        &self.basis.local
    }
    pub fn versions(&self) -> &[Version] {
        &self.versions
    }
}
#[derive(Clone, Debug)]
pub struct Page {
    pub reviews: Vec<Review>,
    pub after: Option<String>,
}
#[derive(Clone, Copy, Debug)]
pub enum Choice {
    Local,
    AddShared(Uuid),
}
pub(crate) fn target(id: Uuid) -> String {
    format!("account:{id}:connection")
}
pub(crate) fn local_change(account: &Account, shared: Uuid) -> anyhow::Result<Change> {
    let mut account = account.clone();
    account.id = shared.to_string();
    metadata::export_account(&account, shared)?
        .into_iter()
        .next()
        .context("Missing account connection")
}
fn ready(state: &history::State) -> anyhow::Result<()> {
    ensure!(
        state.initialized && !state.removed && state.ready == 0 && state.waiting == 0,
        "Finish checking shared history before reviewing accounts."
    );
    Ok(())
}
async fn versions(replica: &Replica, id: Uuid) -> anyhow::Result<Vec<history::Version>> {
    let mut versions = Vec::new();
    let mut after = None;
    loop {
        let page = replica.versions(target(id), after).await?;
        if page.is_empty() {
            break;
        }
        ensure!(
            versions.len() + page.len() <= shep_profile_core::MAX_PARENTS,
            "Too many connection versions need review. Update Shep before resolving them."
        );
        after = page.last().map(|v| v.operation);
        versions.extend(page);
    }
    Ok(versions)
}
async fn live_account(replica: &Replica, id: Uuid) -> anyhow::Result<bool> {
    Ok(replica
        .versions(history::target(&Action::AccountRemoved { id }), None)
        .await?
        .is_empty())
}
pub async fn prepare(
    store: &Store,
    replica: &Replica,
    after: Option<String>,
) -> anyhow::Result<Page> {
    let state = replica.state().await?;
    ready(&state)?;
    let (bases, next) = store
        .profile_account_review_bases(replica.binding().clone(), after)
        .await?;
    let mut reviews = Vec::new();
    for basis in bases {
        // Removal has its own decision; do not revive its hidden connection.
        if !live_account(replica, basis.shared).await? {
            continue;
        }
        let local = local_change(&basis.local, basis.shared)?;
        let actual = versions(replica, basis.shared).await?;
        let mut candidates = Vec::new();
        let mut same = false;
        for version in actual {
            let change = replica
                .value(target(basis.shared), version.operation)
                .await?;
            let Action::AccountConnection { account } = &change.action else {
                anyhow::bail!("The shared account identity changed. Refresh its review.");
            };
            ensure!(
                account.id == basis.shared,
                "The shared connection belongs to another account."
            );
            same = state::normalized(change.clone()) == local;
            candidates.push(Version {
                operation: version.operation,
                account: metadata::review_account(account, &basis.local.name)?,
            });
        }
        if candidates.is_empty() || candidates.len() == 1 && same && basis.pending.is_none() {
            continue;
        }
        reviews.push(Review {
            basis,
            revision: state.revision,
            versions: candidates,
        });
    }
    Ok(Page {
        reviews,
        after: next,
    })
}

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
    ensure!(
        live_account(replica, review.basis.shared).await?,
        "This shared account was removed. Keep the local setup and refresh its review."
    );
    let actual = versions(replica, review.basis.shared).await?;
    ensure!(
        current.revision == review.revision
            && actual
                .iter()
                .map(|v| v.operation)
                .eq(review.versions.iter().map(|v| v.operation)),
        "Shared connections changed while this review was open. Refresh before choosing."
    );
    let operation = match choice {
        Choice::Local => {
            review
                .versions
                .first()
                .context("No shared connection version remains")?
                .operation
        }
        Choice::AddShared(id) => {
            ensure!(
                review.versions.iter().any(|v| v.operation == id),
                "Choose a connection from this review."
            );
            id
        }
    };
    let mut change = replica
        .value(target(review.basis.shared), operation)
        .await?;
    let Action::AccountConnection { account } = &change.action else {
        anyhow::bail!("The connection identity changed.");
    };
    metadata::review_account(account, &review.basis.local.name)?;
    if matches!(choice, Choice::Local) {
        change.action = local_change(&review.basis.local, review.basis.shared)?.action;
    }
    let pending = store
        .reserve_profile_account_review(review, change, matches!(choice, Choice::AddShared(_)))
        .await?;
    let receipt = replica.admit_local(pending).await?;
    store.acknowledge_profile_change(receipt).await
}

pub(crate) fn pending(review: &Review, change: Change, native_revision: u64) -> Pending {
    Pending {
        operation: Uuid::new_v4(),
        expected_revision: review.revision,
        local: state::normalized(change.clone()),
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
    pub(crate) fn fixture(name: &str) -> Self {
        let operation = shep_profile_core::Operation::decode(include_bytes!(
            "../../tests/support/profile-operation.json"
        ))
        .unwrap();
        let Action::AccountConnection { account } = &operation.changes[0].action else {
            panic!("connection fixture")
        };
        let mut local = metadata::review_account(account, name).unwrap();
        local.id = Uuid::new_v4().to_string();
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
                connections_revision: 1,
                local: local.clone(),
                shared: account.id,
                native_revision: 1,
                pending: None,
            },
            revision: 1,
            versions: vec![Version {
                operation: Uuid::new_v4(),
                account: local,
            }],
        }
    }
}
