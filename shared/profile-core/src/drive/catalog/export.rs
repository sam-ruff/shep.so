use super::*;

/// Frozen source metadata for a reviewed transfer into an independently owned
/// local journal. This is a device-local checkpoint, not a portable database.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub binding: Binding,
    pub profile: Profile,
}
impl Catalog {
    pub(super) fn latest_snapshot(&mut self, profile: Uuid, generation: Uuid) -> Result<Snapshot> {
        let summary: String = self
            .db
            .query_row(
                "SELECT summary FROM profiles WHERE key=?",
                [format!("{profile}:{generation}")],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::Changed)?;
        let latest: Profile = serde_json::from_str(&summary).map_err(|_| Error::Storage)?;
        // Keep all completed-scan, identity and independent journal checks in
        // the same owning command as the lookup of this exact profile.
        self.snapshot(profile, generation, latest.revision)
    }

    pub(super) fn snapshot(
        &mut self,
        profile: Uuid,
        generation: Uuid,
        expected_revision: u64,
    ) -> Result<Snapshot> {
        let state = self.state()?;
        if state.phase != Phase::Complete || state.error.is_some() || state.pending != 0 {
            return Err(Error::Failed);
        }
        let binding = self.scope.binding(profile, generation);
        let key = format!("{profile}:{generation}");
        let summary: String = self
            .db
            .query_row("SELECT summary FROM profiles WHERE key=?", [&key], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or(Error::Changed)?;
        let profile: Profile = serde_json::from_str(&summary).map_err(|_| Error::Storage)?;
        if profile.revision != expected_revision {
            return Err(Error::Changed);
        }
        if !profile.initialized || profile.removed {
            return Err(history::Error::Incomplete.into());
        }
        // An independent history receipt may have committed before its summary.
        // Such a partial catalog receipt is not a frozen, reviewable source.
        let current = self.journal(binding.clone())?.overview()?;
        if Profile::from_overview(&binding, current) != profile {
            return Err(Error::Changed);
        }
        Ok(Snapshot { binding, profile })
    }
    pub(super) fn source_device(&mut self, source: Snapshot) -> Result<Uuid> {
        let current = self.snapshot(
            source.profile.profile,
            source.profile.generation,
            source.profile.revision,
        )?;
        if current.binding != source.binding || current.profile != source.profile {
            return Err(Error::Changed);
        }
        Ok(self.journal(current.binding)?.state()?.device)
    }

    // A surviving observation history is not proof that its originals remain
    // in the current verified provider inventory after catalog recovery.
    fn require_original(&self, binding: &Binding, record: &history::Record) -> Result<()> {
        let digest: Option<String> = self.db.query_row(
            "SELECT sha256 FROM files WHERE profile=? AND generation=? AND operation=? AND verified=1",
            params![binding.profile.to_string(),binding.generation.to_string(),record.operation.to_string()],
            |r|r.get(0)).optional()?;
        let digest = digest.ok_or(Error::Missing)?;
        if record.record.len() > crate::MAX_RECORD_BYTES
            || wire::sha256(record.record.as_bytes()) != digest
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
    pub(super) fn verify_original(
        &mut self,
        source: Snapshot,
        record: history::Record,
    ) -> Result<()> {
        let current = self.snapshot(
            source.profile.profile,
            source.profile.generation,
            source.profile.revision,
        )?;
        if current.binding != source.binding || current.profile != source.profile {
            return Err(Error::Changed);
        }
        self.require_original(&source.binding, &record)
    }

    pub(super) fn export_record(
        &mut self,
        source: Snapshot,
        after: u64,
    ) -> Result<Option<history::Record>> {
        let expected = self
            .scope
            .binding(source.profile.profile, source.profile.generation);
        if source.binding != expected {
            return Err(Error::Binding);
        }
        let current = self.snapshot(
            source.profile.profile,
            source.profile.generation,
            source.profile.revision,
        )?;
        if current.profile != source.profile {
            return Err(Error::Changed);
        }
        let record = self
            .journal(expected)?
            .export_record(source.profile.revision, after)?;
        if let Some(record) = &record {
            self.require_original(&source.binding, record)?;
        }
        Ok(record)
    }
}
