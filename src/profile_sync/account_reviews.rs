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
/// A local account that resembles a new shared definition. Only an exact
/// connection match may reuse this device's credentials and mail.
#[derive(Clone, Debug)]
pub struct Match {
    pub account: Account,
    pub exact: bool,
    pub(crate) native_revision: u64,
}
/// A shared account that arrived after enrollment and is still unmapped here.
#[derive(Clone, Debug)]
pub struct Link {
    pub operation: Uuid,
    pub account: Account,
    /// Exact shared change, retaining optional fields for the local basis.
    pub(crate) change: Change,
    pub(crate) revision: u64,
    pub(crate) name: Option<Named>,
    pub matches: Vec<Match>,
}
#[derive(Clone, Debug)]
pub(crate) struct Named {
    pub change: Change,
    pub revision: u64,
}
impl Link {
    pub fn linkable(&self) -> bool {
        self.matches.iter().any(|m| m.exact)
    }
}
/// Raw connection candidate read from history before local matching.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub shared: Uuid,
    pub operation: Uuid,
    pub change: Change,
    pub revision: u64,
    pub name: Option<Named>,
}
#[derive(Clone, Debug)]
pub struct Review {
    pub(crate) basis: Basis,
    pub(crate) revision: u64,
    versions: Vec<Version>,
    removals: Vec<Uuid>,
    link: Option<Link>,
}
impl Review {
    pub fn local(&self) -> &Account {
        &self.basis.local
    }
    pub fn removed(&self) -> bool {
        !self.removals.is_empty()
    }
    pub fn versions(&self) -> &[Version] {
        &self.versions
    }
    pub fn link(&self) -> Option<&Link> {
        self.link.as_ref()
    }
    pub fn shared(&self) -> Uuid {
        self.basis.shared
    }
    /// The native account whose sync ownership a choice must hold.
    pub fn affected_account<'a>(&'a self, choice: &'a Choice) -> &'a str {
        match choice {
            Choice::LinkExisting(id) => id,
            _ => &self.basis.local.id,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Page {
    pub reviews: Vec<Review>,
    pub after: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Choice {
    Local,
    AddShared(Uuid),
    KeepRemovedLocal,
    LinkExisting(String),
    AddNew,
    KeepLocal,
}
const LINK_CURSOR: &str = "link:";
pub(crate) fn target(id: Uuid) -> String {
    format!("account:{id}:connection")
}
fn name_target(id: Uuid) -> String {
    history::target(&Action::AccountName {
        id,
        name: String::new(),
    })
}
fn connection_target(target: &str) -> Option<Uuid> {
    target
        .strip_prefix("account:")?
        .strip_suffix(":connection")
        .and_then(|id| Uuid::parse_str(id).ok())
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
async fn target_versions(
    replica: &Replica,
    target: String,
) -> anyhow::Result<Vec<history::Version>> {
    let mut versions = Vec::new();
    let mut after = None;
    loop {
        let page = replica.versions(target.clone(), after).await?;
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
async fn versions(replica: &Replica, id: Uuid) -> anyhow::Result<Vec<history::Version>> {
    target_versions(replica, target(id)).await
}
async fn removals(replica: &Replica, id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    Ok(
        target_versions(replica, history::target(&Action::AccountRemoved { id }))
            .await?
            .into_iter()
            .map(|version| version.operation)
            .collect(),
    )
}
async fn live_account(replica: &Replica, id: Uuid) -> anyhow::Result<bool> {
    Ok(removals(replica, id).await?.is_empty())
}
/// Unmapped live shared connections after the cursor, oldest target first.
/// Conflicting definitions are left to the ordinary cycle report.
async fn link_candidates(
    store: &Store,
    replica: &Replica,
    after: Option<&str>,
    room: usize,
) -> anyhow::Result<(Vec<Candidate>, Option<String>)> {
    let replication = store.profile_replication(replica.binding().clone()).await?;
    let mapped: std::collections::BTreeSet<Uuid> = replication.accounts.values().copied().collect();
    let mut cursor = Some(
        after
            .and_then(|after| after.strip_prefix(LINK_CURSOR))
            .and_then(|id| Uuid::parse_str(id).ok())
            .map_or_else(|| "account:".to_string(), target),
    );
    let mut candidates = Vec::new();
    'pages: loop {
        let fields = replica.fields(cursor.clone()).await?;
        let Some(last) = fields.last() else {
            break;
        };
        cursor = Some(last.target.clone());
        for field in fields {
            if !field.target.starts_with("account:") {
                break 'pages;
            }
            let Some(shared) = connection_target(&field.target) else {
                continue;
            };
            if mapped.contains(&shared)
                || replication.suppressed.contains(&shared)
                || field.conflict
                || field.versions != 1
            {
                continue;
            }
            if candidates.len() == room {
                // The cursor names the last included definition, or the start
                // of this section when mapped reviews already filled the page.
                let next = candidates.last().map_or_else(
                    || LINK_CURSOR.to_string(),
                    |c: &Candidate| format!("{LINK_CURSOR}{}", c.shared),
                );
                return Ok((candidates, Some(next)));
            }
            let version = versions(replica, shared)
                .await?
                .into_iter()
                .next()
                .context("The shared connection disappeared. Refresh this review.")?;
            let change = replica
                .value(field.target.clone(), version.operation)
                .await?;
            let names = target_versions(replica, name_target(shared)).await?;
            let name = match names.as_slice() {
                [only] => Some(Named {
                    change: replica.value(name_target(shared), only.operation).await?,
                    revision: replica
                        .fields(Some(format!("account:{shared}:")))
                        .await?
                        .into_iter()
                        .find(|f| f.target == name_target(shared))
                        .map_or(field.revision, |f| f.revision),
                }),
                _ => None,
            };
            candidates.push(Candidate {
                shared,
                operation: version.operation,
                change,
                revision: field.revision,
                name,
            });
        }
    }
    Ok((candidates, None))
}

pub async fn prepare(
    store: &Store,
    replica: &Replica,
    after: Option<String>,
) -> anyhow::Result<Page> {
    let state = replica.state().await?;
    ready(&state)?;
    let linking = after.as_deref().is_some_and(|a| a.starts_with(LINK_CURSOR));
    let (bases, next) = if linking {
        (vec![], None)
    } else {
        store
            .profile_account_review_bases(replica.binding().clone(), after.clone())
            .await?
    };
    let mut reviews = Vec::new();
    for basis in bases {
        // A remote tombstone never silently deletes native mail or credentials.
        let removals = removals(replica, basis.shared).await?;
        if !removals.is_empty() {
            reviews.push(Review {
                basis,
                revision: state.revision,
                versions: vec![],
                removals,
                link: None,
            });
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
            removals: vec![],
            link: None,
        });
    }
    if next.is_some() {
        return Ok(Page {
            reviews,
            after: next,
        });
    }
    // New shared definitions resembling an unmapped native account wait here
    // rather than silently becoming another reconnecting account.
    let (candidates, next) = link_candidates(
        store,
        replica,
        linking.then_some(after.as_deref()).flatten(),
        PAGE_SIZE - reviews.len(),
    )
    .await?;
    for (basis, link) in store
        .profile_account_link_bases(replica.binding().clone(), candidates)
        .await?
    {
        reviews.push(Review {
            basis,
            revision: state.revision,
            versions: vec![],
            removals: vec![],
            link: Some(link),
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
    if review.removed() {
        ensure!(
            matches!(choice, Choice::KeepRemovedLocal),
            "Review this account's removal before choosing."
        );
        ensure!(
            current.revision == review.revision
                && removals(replica, review.basis.shared).await? == review.removals,
            "Shared history changed. Refresh this removal review before choosing."
        );
        return store.keep_removed_profile_account(review).await;
    }
    if let Some(link) = &review.link {
        ensure!(
            matches!(
                choice,
                Choice::LinkExisting(_) | Choice::AddNew | Choice::KeepLocal
            ),
            "Choose how this device should treat the new shared account."
        );
        ensure!(
            live_account(replica, review.basis.shared).await?,
            "This shared account was removed. Refresh its review."
        );
        let actual = versions(replica, review.basis.shared).await?;
        ensure!(
            current.revision == review.revision
                && actual.len() == 1
                && actual[0].operation == link.operation
                && replica
                    .value(target(review.basis.shared), link.operation)
                    .await?
                    == link.change,
            "Shared history changed while this review was open. Refresh before choosing."
        );
        return store.resolve_profile_account_link(review, choice).await;
    }
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
        Choice::KeepRemovedLocal => {
            anyhow::bail!("This account has not been removed from the shared profile.")
        }
        Choice::LinkExisting(_) | Choice::AddNew | Choice::KeepLocal => {
            anyhow::bail!("This account is already part of the shared profile on this device.")
        }
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
            removals: vec![],
            versions: vec![Version {
                operation: Uuid::new_v4(),
                account: local,
            }],
            link: None,
        }
    }
    pub(crate) fn link_fixture(name: &str, exact: bool) -> Self {
        let mut review = Self::fixture(name);
        let change = shep_profile_core::Operation::decode(include_bytes!(
            "../../tests/support/profile-operation.json"
        ))
        .unwrap()
        .changes
        .remove(0);
        let Action::AccountConnection { account } = &change.action else {
            panic!("connection fixture")
        };
        let shared = metadata::review_account(account, "Shared name").unwrap();
        review.versions.clear();
        review.link = Some(Link {
            operation: Uuid::new_v4(),
            account: shared,
            change,
            revision: 1,
            name: None,
            matches: vec![Match {
                account: review.basis.local.clone(),
                exact,
                native_revision: 1,
            }],
        });
        review
    }
}
