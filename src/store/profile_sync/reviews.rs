use super::*;
use crate::profile_sync::{
    metadata,
    reviews::{self, Basis, Review},
    state as replication,
};
use shep_profile_core::{Action, Change, history};

impl Store {
    pub(crate) async fn profile_setting_review_bases(
        &self,
        binding: history::Binding,
    ) -> anyhow::Result<Vec<Basis>> {
        self.run(move |c| {
            let (enrollment, selection) = state::selected(c)?;
            anyhow::ensure!(
                selection.binding == binding
                    && enrollment.options.enabled
                    && enrollment.options.settings,
                "Enable preference sync for this profile before reviewing changes."
            );
            let current = state::read(c, &binding)?;
            let revisions = state::native_revisions(c, &current)?;
            let prefs: Preferences = get(c, "preferences")?;
            let mut bases = Vec::new();
            for &key in metadata::SETTINGS {
                let target = history::target(&Action::SettingRemoved { key });
                let local =
                    metadata::setting_value(key, &prefs).context("Unsupported preference")?;
                let native_revision = revisions.get(&target).copied().unwrap_or_default();
                let field = current.fields.get(&target);
                let local_change = Change {
                    action: Action::Setting {
                        key,
                        value: local.clone(),
                    },
                    extra: Default::default(),
                };
                let pending = current
                    .pending
                    .iter()
                    .chain(current.deferred.values())
                    .find(|p| p.target() == target)
                    .map(|p| p.operation);
                bases.push(Basis {
                    binding: binding.clone(),
                    enrollment_revision: enrollment.revision,
                    google_revision: prefs.google_lifecycle.revision,
                    key,
                    local,
                    native_revision,
                    pending,
                    changed: field.and_then(|f| f.local.as_ref()) != Some(&local_change)
                        || native_revision > field.map_or(0, |f| f.native_revision),
                });
            }
            Ok(bases)
        })
        .await
    }

    pub(crate) async fn reserve_profile_setting_review(
        &self,
        review: Review,
        change: Change,
    ) -> anyhow::Result<replication::Pending> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = state::selected(&tx)?;
            let mut current = state::read(&tx, &selection.binding)?;
            let mut prefs: Preferences = get(&tx, "preferences")?;
            let basis = &review.basis;
            let target = history::target(&change.action);
            let expected_target = history::target(&Action::SettingRemoved { key: basis.key });
            let revisions = state::native_revisions(&tx, &current)?;
            let pending = current.pending.iter().chain(current.deferred.values()).find(|p| p.target() == target).map(|p| p.operation);
            anyhow::ensure!(selection.binding == basis.binding && enrollment.revision == basis.enrollment_revision
                && enrollment.options.enabled && enrollment.options.settings
                && prefs.google_lifecycle.revision == basis.google_revision,
                "Profile sync choices changed. Refresh this review before choosing.");
            anyhow::ensure!(target == expected_target && review.revision >= current.revision
                && metadata::setting_value(basis.key, &prefs).as_ref() == Some(&basis.local)
                && revisions.get(&target).copied().unwrap_or_default() == basis.native_revision
                && pending == basis.pending,
                "This preference changed while the review was open. Refresh it to keep your newer choice.");
            anyhow::ensure!(current.pending.as_ref().is_none_or(|p| p.target() == target),
                "Another preference is being saved. Wait for it, then refresh this review.");
            let before = prefs.clone();
            metadata::apply_setting(&mut prefs, &change)?;
            prefs.validate()?;
            if prefs != before {
                put(&tx, "preferences", &prefs)?;
                state::record_native_preferences(&tx, &before, &prefs, get(&tx, "preferences_revision")?)?;
            }
            let native_revision = state::native_revisions(&tx, &current)?.get(&target).copied().unwrap_or_default();
            let pending = reviews::pending(&review, change, native_revision);
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
