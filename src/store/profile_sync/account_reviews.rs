use super::*;
use crate::profile_sync::{
    account_reviews::{self, Basis, Candidate, Choice, Link, Review},
    join::{self, links},
    metadata, state as replication,
};
use shep_profile_core::{Action, Change, history};

fn native_key(kind: &str, local: &str) -> String {
    format!("local-account-{kind}:{local}")
}

impl Store {
    /// Match unmapped shared definitions against this device's unmapped
    /// accounts under one frozen set of profile/Google/connection generations.
    pub(crate) async fn profile_account_link_bases(
        &self,
        binding: history::Binding,
        candidates: Vec<Candidate>,
    ) -> anyhow::Result<Vec<(Basis, Link)>> {
        self.run(move |c| {
            let (enrollment, selection) = state::selected(c)?;
            anyhow::ensure!(
                selection.binding == binding
                    && enrollment.options.enabled
                    && enrollment.options.accounts,
                "Enable account sync for this profile before reviewing connections."
            );
            let current = state::read(c, &binding)?;
            let native = state::native_revisions(c, &current)?;
            let prefs: Preferences = get(c, "preferences")?;
            let connections_revision = get(c, "connections_revision")?;
            let accounts: Vec<Account> = get(c, "accounts")?;
            let mut links = Vec::new();
            for candidate in candidates {
                if current.accounts.values().any(|s| *s == candidate.shared)
                    || current.suppressed.contains(&candidate.shared)
                {
                    continue;
                }
                let Action::AccountConnection { account } = &candidate.change.action else {
                    continue;
                };
                anyhow::ensure!(
                    account.id == candidate.shared,
                    "The shared connection belongs to another account."
                );
                let name = candidate
                    .name
                    .as_ref()
                    .and_then(|named| match &named.change.action {
                        Action::AccountName { name, .. } => Some(name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| account.email.clone());
                let Ok(shared) = metadata::review_account(account, &name) else {
                    continue;
                };
                let matches = continuous::link_matches_with(&current, &accounts, &shared, &native);
                let Some(first) = matches.first() else {
                    continue;
                };
                links.push((
                    Basis {
                        binding: binding.clone(),
                        enrollment_revision: enrollment.revision,
                        google_revision: prefs.google_lifecycle.revision,
                        connections_revision,
                        local: first.account.clone(),
                        shared: candidate.shared,
                        native_revision: first.native_revision,
                        pending: None,
                    },
                    Link {
                        operation: candidate.operation,
                        account: shared,
                        change: candidate.change,
                        revision: candidate.revision,
                        name: candidate.name,
                        matches,
                    },
                ));
            }
            Ok(links)
        })
        .await
    }

    /// Link, add or keep local in one transaction. Linking reuses the exact
    /// native row and keychain slot and publishes no new definition; keeping
    /// local records durable suppression of the shared identity.
    pub(crate) async fn resolve_profile_account_link(
        &self,
        review: Review,
        choice: Choice,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = state::selected(&tx)?;
            let mut current = state::read(&tx, &selection.binding)?;
            let prefs: Preferences = get(&tx, "preferences")?;
            let mut accounts: Vec<Account> = get(&tx, "accounts")?;
            let basis = &review.basis;
            let link = review
                .link()
                .context("This review does not offer account linking.")?;
            let target = account_reviews::target(basis.shared);
            let native = state::native_revisions(&tx, &current)?;
            anyhow::ensure!(
                selection.binding == basis.binding
                    && enrollment.revision == basis.enrollment_revision
                    && enrollment.options.enabled
                    && enrollment.options.accounts
                    && prefs.google_lifecycle.revision == basis.google_revision,
                "Profile sync choices changed. Refresh this account review."
            );
            let unchanged = |m: &account_reviews::Match| {
                accounts.iter().any(|account| account == &m.account)
                    && !current.accounts.contains_key(&m.account.id)
                    && native
                        .get(&native_key("connection", &m.account.id))
                        .copied()
                        .unwrap_or_default()
                        == m.native_revision
            };
            anyhow::ensure!(
                review.revision >= current.revision
                    && get::<u64>(&tx, "connections_revision")? == basis.connections_revision
                    && !current.accounts.values().any(|s| *s == basis.shared)
                    && !current.suppressed.contains(&basis.shared)
                    && current
                        .pending
                        .iter()
                        .chain(current.deferred.values())
                        .all(|p| p.target() != target)
                    && link.matches.iter().all(unchanged),
                "The local account changed while this review was open. Refresh to keep your newer settings."
            );
            let field = |change: &Change, revision: u64, native_revision: u64| replication::Field {
                local: Some(replication::normalized(change.clone())),
                remote: Some(change.clone()),
                revision,
                native_revision,
            };
            match choice {
                Choice::KeepLocal => {
                    current.suppressed.insert(basis.shared);
                }
                Choice::AddNew => {
                    let mut reconnect: join::Reconnect = get(&tx, join::RECONNECT_KEY)?;
                    continuous::add_shared_account(
                        &tx,
                        &mut current,
                        &mut accounts,
                        &mut reconnect,
                        link.account.clone(),
                        basis.shared,
                    )?;
                    current
                        .fields
                        .insert(target, field(&link.change, link.revision, 0));
                    if let Some(named) = &link.name {
                        current.fields.insert(
                            history::target(&named.change.action),
                            field(&named.change, named.revision, 0),
                        );
                    }
                    put(&tx, "accounts", &accounts)?;
                    put(&tx, join::RECONNECT_KEY, &reconnect)?;
                    connections::changed(&tx)?;
                }
                Choice::LinkExisting(id) => {
                    let chosen = link
                        .matches
                        .iter()
                        .find(|m| m.account.id == id && m.exact)
                        .context("Choose an account whose connection matches exactly.")?;
                    // The local export must equal the shared definition exactly,
                    // so no message UID or password is redirected elsewhere.
                    anyhow::ensure!(
                        links::compatible(&chosen.account, &link.account)?
                            && replication::normalized(link.change.clone())
                                == account_reviews::local_change(&chosen.account, basis.shared)?,
                        "The linked connection changed. Refresh the review before linking."
                    );
                    current.local_only.remove(&id);
                    current.accounts.insert(id.clone(), basis.shared);
                    current.fields.insert(
                        target,
                        field(&link.change, link.revision, chosen.native_revision),
                    );
                    if let Some(named) = &link.name {
                        let name_revision = native
                            .get(&native_key("name", &id))
                            .copied()
                            .unwrap_or_default();
                        current.fields.insert(
                            history::target(&named.change.action),
                            field(&named.change, named.revision, name_revision),
                        );
                    }
                }
                _ => anyhow::bail!("Choose how this device should treat the new shared account."),
            }
            current.revision = current.revision.max(review.revision);
            current.validate()?;
            put(&tx, replication::STORAGE_KEY, &current)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn profile_account_review_bases(
        &self,
        binding: history::Binding,
        after: Option<String>,
    ) -> anyhow::Result<(Vec<Basis>, Option<String>)> {
        self.run(move |c| {
            let (enrollment, selection) = state::selected(c)?;
            anyhow::ensure!(
                selection.binding == binding
                    && enrollment.options.enabled
                    && enrollment.options.accounts,
                "Enable account sync for this profile before reviewing connections."
            );
            let current = state::read(c, &binding)?;
            let revisions = state::native_revisions(c, &current)?;
            let prefs: Preferences = get(c, "preferences")?;
            let connections_revision = get(c, "connections_revision")?;
            let accounts: Vec<Account> = get(c, "accounts")?;
            let mut bases = Vec::new();
            let mut more = false;
            for (local, shared) in &current.accounts {
                if after.as_ref().is_some_and(|after| local <= after)
                    || current.suppressed.contains(shared)
                {
                    continue;
                }
                let Some(account) = accounts.iter().find(|account| &account.id == local) else {
                    continue;
                };
                if bases.len() == account_reviews::PAGE_SIZE {
                    more = true;
                    break;
                }
                let target = account_reviews::target(*shared);
                bases.push(Basis {
                    binding: binding.clone(),
                    enrollment_revision: enrollment.revision,
                    google_revision: prefs.google_lifecycle.revision,
                    connections_revision,
                    local: account.clone(),
                    shared: *shared,
                    native_revision: revisions.get(&target).copied().unwrap_or_default(),
                    pending: current
                        .pending
                        .iter()
                        .chain(current.deferred.values())
                        .find(|p| p.target() == target)
                        .map(|p| p.operation),
                });
            }
            let next = more.then(|| bases.last().expect("full page").local.id.clone());
            Ok((bases, next))
        })
        .await
    }

    /// Suppression is local and durable: keep the account, mail and credential
    /// identity intact without republishing the remotely removed account.
    pub(crate) async fn keep_removed_profile_account(&self, review: Review) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = state::selected(&tx)?;
            let mut current = state::read(&tx, &selection.binding)?;
            let prefs: Preferences = get(&tx, "preferences")?;
            let accounts: Vec<Account> = get(&tx, "accounts")?;
            let basis = &review.basis;
            let target = account_reviews::target(basis.shared);
            let native = state::native_revisions(&tx, &current)?;
            let pending = current
                .pending
                .iter()
                .chain(current.deferred.values())
                .find(|p| p.target() == target)
                .map(|p| p.operation);
            anyhow::ensure!(
                review.removed()
                    && selection.binding == basis.binding
                    && enrollment.revision == basis.enrollment_revision
                    && enrollment.options.enabled
                    && enrollment.options.accounts
                    && prefs.google_lifecycle.revision == basis.google_revision,
                "Profile sync choices changed. Refresh this removal review."
            );
            anyhow::ensure!(
                review.revision >= current.revision
                    && get::<u64>(&tx, "connections_revision")? == basis.connections_revision
                    && current.accounts.get(&basis.local.id) == Some(&basis.shared)
                    && !current.suppressed.contains(&basis.shared)
                    && accounts.iter().any(|account| account == &basis.local)
                    && native.get(&target).copied().unwrap_or_default() == basis.native_revision
                    && pending == basis.pending,
                "The local account changed. Refresh this removal review to keep your newer choices."
            );
            connections::allow(&tx, ConnectionKind::Account, &basis.local.id)?;
            let name = history::target(&Action::AccountName {
                id: basis.shared,
                name: String::new(),
            });
            if current
                .pending
                .as_ref()
                .is_some_and(|p| p.target() == target || p.target() == name)
            {
                current.pending = None;
            }
            current.deferred.remove(&target);
            current.deferred.remove(&name);
            current.suppressed.insert(basis.shared);
            current.revision = current.revision.max(review.revision);
            current.validate()?;
            put(&tx, replication::STORAGE_KEY, &current)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn reserve_profile_account_review(
        &self,
        review: Review,
        change: Change,
        add_shared: bool,
    ) -> anyhow::Result<replication::Pending> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = state::selected(&tx)?;
            let mut current = state::read(&tx, &selection.binding)?;
            let prefs: Preferences = get(&tx, "preferences")?;
            let mut accounts: Vec<Account> = get(&tx, "accounts")?;
            let basis = &review.basis;
            let target = account_reviews::target(basis.shared);
            let native = state::native_revisions(&tx, &current)?;
            let pending = current.pending.iter().chain(current.deferred.values()).find(|p| p.target() == target).map(|p| p.operation);
            anyhow::ensure!(selection.binding == basis.binding && enrollment.revision == basis.enrollment_revision
                && enrollment.options.enabled && enrollment.options.accounts
                && prefs.google_lifecycle.revision == basis.google_revision,
                "Profile sync choices changed. Refresh this connection review.");
            anyhow::ensure!(history::target(&change.action) == target && review.revision >= current.revision
                && get::<u64>(&tx, "connections_revision")? == basis.connections_revision
                && current.accounts.get(&basis.local.id) == Some(&basis.shared)
                && !current.suppressed.contains(&basis.shared)
                && accounts.iter().any(|account| account == &basis.local)
                && native.get(&target).copied().unwrap_or_default() == basis.native_revision
                && pending == basis.pending,
                "The local account changed while this review was open. Refresh to keep your newer settings.");
            anyhow::ensure!(current.pending.as_ref().is_none_or(|p| p.target() == target),
                "Another profile change is being saved. Wait for it, then refresh this review.");
            connections::allow(&tx, ConnectionKind::Account, &basis.local.id)?;
            let Action::AccountConnection { account } = &change.action else { anyhow::bail!("Choose a reviewed account connection."); };
            anyhow::ensure!(account.id == basis.shared, "The reviewed connection identity changed.");
            let mut imported = metadata::review_account(account, &basis.local.name)?;
            let local = account_reviews::local_change(&basis.local, basis.shared)?;
            let mut native_revision = basis.native_revision;
            if add_shared && replication::normalized(change.clone()) != local {
                // Never reuse a cache/server identity or credential slot for a
                // different endpoint. The previous account remains local-only.
                imported.id = uuid::Uuid::new_v4().to_string();
                connections::allow(&tx, ConnectionKind::Account, &imported.id)?;
                let previous = accounts.iter_mut().find(|account| account.id == basis.local.id).expect("checked account");
                let mut name = previous.name.clone();
                while name.len() > 230 { name.pop(); }
                previous.name = format!("{name} (previous setup)");
                previous.validate()?;
                current.accounts.remove(&basis.local.id);
                current.local_only.insert(basis.local.id.clone());
                current.accounts.insert(imported.id.clone(), basis.shared);
                let mut reconnect: crate::profile_sync::join::Reconnect = get(&tx, crate::profile_sync::join::RECONNECT_KEY)?;
                reconnect.insert(imported.id.clone());
                if !imported.sent_folder.is_empty() {
                    tx.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?)", params![imported.id, imported.sent_folder])?;
                }
                accounts.push(imported);
                put(&tx, "accounts", &accounts)?;
                put(&tx, crate::profile_sync::join::RECONNECT_KEY, &reconnect)?;
                connections::changed(&tx)?;
                // This new native identity has no unacknowledged local edits.
                native_revision = 0;
            } else {
                anyhow::ensure!(replication::normalized(change.clone()) == local,
                    "The chosen local connection differs from the review.");
            }
            let pending = account_reviews::pending(&review, change, native_revision);
            current.revision = current.revision.max(review.revision);
            current.deferred.remove(&target);
            current.pending = Some(pending.clone());
            current.validate()?;
            put(&tx, replication::STORAGE_KEY, &current)?;
            tx.commit()?;
            Ok(pending)
        }).await
    }
}
