use super::*;
use crate::profile_sync::{
    account_reviews::{self, Basis, Review},
    metadata, state as replication,
};
use shep_profile_core::{Action, Change, history};

impl Store {
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
