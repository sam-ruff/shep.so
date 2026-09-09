use super::*;
use crate::profiles::{
    preference_state::PreferenceState,
    preferences::{SUPPORTED, export},
};
use shep_profile_core::SettingKey;
use std::collections::BTreeSet;

pub(super) fn schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_preference_revisions(field TEXT PRIMARY KEY, revision INTEGER NOT NULL);")?;
    Ok(())
}
pub(crate) fn mark(db: &Connection, field: SettingKey, revision: u64) -> anyhow::Result<()> {
    let revision: i64 = revision
        .try_into()
        .context("Preference revision overflow")?;
    db.execute("INSERT INTO profile_preference_revisions(field,revision) VALUES(?,?) ON CONFLICT(field) DO UPDATE SET revision=excluded.revision",params![serde_json::to_string(&field)?,revision])?;
    Ok(())
}
pub(crate) fn changed(db: &Connection, next: &Preferences, revision: u64) -> anyhow::Result<()> {
    let old = export(&get::<Preferences>(db, "preferences")?)?;
    for (field, value) in export(next)? {
        if old.get(&field) != Some(&value) {
            mark(db, field, revision)?;
        }
    }
    Ok(())
}
pub(crate) fn state(db: &Connection) -> anyhow::Result<PreferenceState> {
    let mut state = PreferenceState {
        values: export(&get::<Preferences>(db, "preferences")?)?,
        revisions: SUPPORTED.into_iter().map(|key| (key, 0)).collect(),
    };
    let mut query = db.prepare("SELECT field,revision FROM profile_preference_revisions")?;
    let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (field, revision) = row?;
        state
            .revisions
            .insert(serde_json::from_str(&field)?, revision.try_into()?);
    }
    Ok(state)
}
impl Store {
    /// User saves carry the portable fields actually edited since the preceding
    /// accepted save. An older whole-window snapshot cannot undo profile fields.
    pub(crate) async fn save_profile_preferences(
        &self,
        mut requested: Preferences,
        fields: BTreeSet<SettingKey>,
    ) -> anyhow::Result<PreferenceSnapshot> {
        self.run(move |db| {
            let tx = db.transaction()?;
            anyhow::ensure!(
                fields.iter().all(|key| SUPPORTED.contains(key)),
                "Unsupported preference edit. Reopen Preferences."
            );
            let mut current: Preferences = get(&tx, "preferences")?;
            for (key, value) in export(&current)? {
                if !fields.contains(&key) {
                    crate::profiles::preferences::apply(&mut requested, key, Some(&value))?;
                }
            }
            merge(&mut current, requested);
            current.validate()?;
            put(&tx, "preferences", &current)?;
            let revision = get(&tx, "preferences_revision")?;
            // Explicit newer intent remains newer when changed back to the same
            // value, or when an intermediate UI save could not enter the queue.
            for key in fields {
                mark(&tx, key, revision)?;
            }
            tx.commit()?;
            Ok(PreferenceSnapshot {
                revision,
                value: current,
            })
        })
        .await
    }
}
pub(super) fn merge(current: &mut Preferences, requested: Preferences) {
    let last_backup = current.last_backup;
    let backup_ready = current.backup_ready;
    let previous_target = crate::backup::BackupTarget::from_preferences(current);
    let connection = current.google_connection_id.clone();
    let lifecycle = current.google_lifecycle;
    let grant = current.google_grant.clone();
    *current = requested;
    current.google_connection_id = connection;
    current.google_lifecycle = lifecycle;
    current.google_grant = grant;
    if (lifecycle.disconnected || !current.google_grant.access.drive_allowed())
        && current.backup_destination == BackupDestination::GoogleDrive
    {
        current.auto_backup = false;
    }
    let same_target = previous_target == crate::backup::BackupTarget::from_preferences(current);
    current.last_backup = if same_target { last_backup } else { None };
    current.backup_ready = same_target && backup_ready;
}
