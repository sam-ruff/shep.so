use super::*;
use crate::profile_sync::join::{self, Applied, Reconnect, Review, Values};
use std::collections::BTreeMap;

impl Store {
    pub(crate) async fn applied_profile_join(
        &self,
        review: uuid::Uuid,
    ) -> anyhow::Result<Option<Snapshot>> {
        self.run(move |c| {
            let saved: Option<Applied> = get(c, join::STORAGE_KEY)?;
            if saved.is_some_and(|s| s.review == review) {
                Ok(Some(snapshot(c)?))
            } else {
                Ok(None)
            }
        })
        .await
    }

    pub(crate) async fn accept_profile_join(
        &self,
        review: Review,
        values: Values,
    ) -> anyhow::Result<Snapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let applied: Option<Applied> = get(&tx,join::STORAGE_KEY)?;
            if applied.as_ref().is_some_and(|a|a.review == review.id) { return snapshot(&tx); }
            let mut enrollment = review_matches(&tx,&review.local)?;
            anyhow::ensure!(enrollment.selection.is_none() && applied.is_none(),
                "This workspace already has a shared profile. Use another local workspace to keep both profiles separate.");
            let mut prefs: Preferences = get(&tx,"preferences")?;
            enrollment::check_google(&prefs,&review.selection)?;
            let options = Options { enabled:true,..enrollment.options };
            options.validate()?;
            anyhow::ensure!(review.selection.origin == enrollment::Origin::Join && review.selection.ready,
                "Review an existing shared profile before joining.");
            anyhow::ensure!(values.accounts.len() == review.accounts && values.settings.len() == review.settings && (options.accounts || values.accounts.is_empty()) && (options.settings || values.settings.is_empty()),
                "The selected profile categories changed. Review the profile again.");
            let mut accounts: Vec<Account> = get(&tx,"accounts")?;
            let mut common = values.account_changes;
            common.extend(values.settings.iter().cloned());
            let mut reconnect: Reconnect = get(&tx,join::RECONNECT_KEY)?;
            let mut mapping = BTreeMap::new();
            for mut account in values.accounts {
                account.validate()?;
                let shared = uuid::Uuid::parse_str(&account.id)?;
                anyhow::ensure!(!shared.is_nil() && !mapping.contains_key(&shared),"The profile repeats an account identity.");
                // A shared identifier must never select an existing keychain
                // slot or silently replace another local account's endpoints.
                let local = uuid::Uuid::new_v4().to_string();
                anyhow::ensure!(!accounts.iter().any(|a|a.id == local),"The new account identity is already in use. Review the profile again.");
                connections::allow(&tx,ConnectionKind::Account,&local)?;
                account.id = local.clone();
                if !account.sent_folder.is_empty() {
                    tx.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?)",params![local,account.sent_folder])?;
                }
                reconnect.insert(local.clone());
                mapping.insert(shared,local);
                accounts.push(account);
            }
            if !mapping.is_empty() {
                put(&tx,"accounts",&accounts)?;
                put(&tx,join::RECONNECT_KEY,&reconnect)?;
                connections::changed(&tx)?;
            }
            let mut changed = false;
            let mut keys = std::collections::BTreeSet::new();
            for change in values.settings {
                use shep_profile_core::Action;
                let key = match &change.action {
                    Action::Setting{key,..}|Action::SettingRemoved{key}=>*key,
                    _=>anyhow::bail!("The shared settings review contains another kind of change."),
                };
                anyhow::ensure!(keys.insert(key),"The shared settings review contains duplicate values.");
                changed |= crate::profile_sync::metadata::apply_setting(&mut prefs,&change)?;
            }
            prefs.validate()?;
            if changed { put(&tx,"preferences",&prefs)?; }
            enrollment.selection = Some(review.selection.clone());
            enrollment.options = options;
            enrollment.last_success = Some(chrono::Utc::now().timestamp());
            enrollment.advance()?;
            enrollment.validate()?;
            put(&tx,STORAGE_KEY,&enrollment)?;
            // The common values and native import commit together. A crash or
            // later edit cannot turn acceptance into a new baseline snapshot.
            let replication = crate::profile_sync::state::State::new(
                review.selection.binding.clone(),review.revision,&accounts,&prefs,
                mapping.iter().map(|(shared,local)|(local.clone(),*shared)).collect(),common)?;
            anyhow::ensure!(get::<Option<crate::profile_sync::state::State>>(&tx,crate::profile_sync::state::STORAGE_KEY)?.is_none(),
                "This workspace already has a profile checkpoint. Review its existing setup.");
            put(&tx,crate::profile_sync::state::STORAGE_KEY,&replication)?;
            put(&tx,join::STORAGE_KEY,&Applied { review:review.id,binding:review.selection.binding,history_revision:review.revision,accounts:mapping })?;
            let result = snapshot(&tx)?;
            tx.commit()?;
            Ok(result)
        }).await
    }

    pub(crate) async fn require_account_reconnected(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let pending: Reconnect = get(c,join::RECONNECT_KEY)?;
            anyhow::ensure!(!pending.contains(&id),"Reconnect this shared account in Preferences → Accounts before receiving or sending mail.");
            Ok(())
        }).await
    }

    pub(crate) async fn accounts_ready_to_sync(&self) -> anyhow::Result<Vec<Account>> {
        self.run(|c| {
            let pending: Reconnect = get(c, join::RECONNECT_KEY)?;
            Ok(get::<Vec<Account>>(c, "accounts")?
                .into_iter()
                .filter(|a| !pending.contains(&a.id))
                .collect())
        })
        .await
    }
}

/// Only called after SaveAccount has persisted the supplied device credential,
/// or when explicitly removing the account. Ordinary refresh cannot clear this.
pub(in crate::store) fn reconnected(c: &Connection, id: &str) -> anyhow::Result<()> {
    let mut pending: Reconnect = get(c, join::RECONNECT_KEY)?;
    if pending.remove(id) {
        put(c, join::RECONNECT_KEY, &pending)?;
    }
    Ok(())
}
