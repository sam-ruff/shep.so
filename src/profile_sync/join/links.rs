//! Explicit reuse of a native account during import. The local keychain identity
//! can only be reused when the reviewed portable connection is unchanged.
use super::*;

pub type Links = BTreeMap<Uuid, String>;

#[derive(Clone, Debug)]
pub struct AccountOffer {
    pub shared: Uuid,
    pub name: String,
    pub email: String,
    pub matches: Vec<LocalAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAccount {
    pub id: String,
    pub name: String,
}

pub(crate) fn compatible(local: &Account, shared: &Account) -> anyhow::Result<bool> {
    // Native IDs deliberately remain local. Compare every portable connection
    // field, including SMTP usernames, security, auth and sent-copy policy.
    let id = Uuid::parse_str(&shared.id)?;
    let mut native = local.clone();
    native.id = shared.id.clone();
    let native = metadata::export_account(&native, id)?;
    let remote = metadata::export_account(shared, id)?;
    Ok(native[0] == remote[0])
}

pub(crate) fn offers(accounts: &[Account], local: &[Account]) -> anyhow::Result<Vec<AccountOffer>> {
    accounts
        .iter()
        .map(|account| {
            let mut matches = Vec::new();
            for candidate in local {
                if compatible(candidate, account).unwrap_or(false) {
                    matches.push(LocalAccount {
                        id: candidate.id.clone(),
                        name: candidate.name.clone(),
                    });
                }
            }
            Ok(AccountOffer {
                shared: Uuid::parse_str(&account.id)?,
                name: account.name.clone(),
                email: account.email.clone(),
                matches,
            })
        })
        .collect()
}

impl Review {
    pub fn account_page(&self, offset: usize) -> &[AccountOffer] {
        let start = offset.min(self.account_offers.len());
        &self.account_offers[start..start.saturating_add(8).min(self.account_offers.len())]
    }

    pub(crate) fn validate_links(&self, links: &Links) -> anyhow::Result<()> {
        let mut used = BTreeSet::new();
        for (shared, local) in links {
            anyhow::ensure!(
                used.insert(local),
                "Choose a different local account for each shared account."
            );
            anyhow::ensure!(
                self.account_offers
                    .iter()
                    .any(|offer| offer.shared == *shared
                        && offer.matches.iter().any(|candidate| candidate.id == *local)),
                "Choose an existing account from the current import review."
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_join_account_pages_are_bounded_and_cannot_link_one_local_account_twice() {
        let mut review = Review {
            id: Uuid::new_v4(),
            automatic: false,
            local: Snapshot {
                enrollment: Enrollment::default(),
                preferences_revision: 0,
                connections_revision: 0,
                google_revision: 0,
                google_identity: "drive:fixture".into(),
                available: true,
                accounts: 1,
                empty_workspace: false,
            },
            selection: Selection {
                binding: history::Binding {
                    namespace: "so.shep".into(),
                    principal: "drive:fixture".into(),
                    profile: Uuid::new_v4(),
                    generation: Uuid::new_v4(),
                },
                name: "Home".into(),
                origin: Origin::Join,
                ready: true,
            },
            revision: 0,
            device: Uuid::new_v4(),
            accounts: 19,
            settings: 0,
            account_offers: Vec::new(),
        };
        for i in 0..19 {
            review.account_offers.push(AccountOffer {
                shared: Uuid::new_v4(),
                name: format!("Account {i}"),
                email: "a@example.test".into(),
                matches: vec![LocalAccount {
                    id: "existing".into(),
                    name: "Existing".into(),
                }],
            });
        }
        assert_eq!(review.account_page(0).len(), 8);
        assert_eq!(review.account_page(8).len(), 8);
        assert_eq!(review.account_page(16).len(), 3);
        assert!(review.account_page(usize::MAX).is_empty());
        let first = review.account_offers[0].shared;
        let second = review.account_offers[1].shared;
        assert!(
            review
                .validate_links(&Links::from([(first, "existing".into())]))
                .is_ok()
        );
        assert!(
            review
                .validate_links(&Links::from([
                    (first, "existing".into()),
                    (second, "existing".into())
                ]))
                .is_err()
        );
        assert!(
            review
                .validate_links(&Links::from([(first, "unoffered".into())]))
                .is_err()
        );
    }
}
