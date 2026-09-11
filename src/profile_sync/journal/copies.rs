//! Durable copy cursor for records exported from the shared discovery catalog
//! into this device's enrolled history. The cursor is bound to the catalog's
//! observation identity and the enrolled history's device; either changing
//! replays every record from the beginning rather than trusting a saved offset.
use super::*;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS catalog_copies(
        identity TEXT NOT NULL,namespace TEXT NOT NULL,
        profile TEXT NOT NULL,generation TEXT NOT NULL,
        source TEXT NOT NULL,device TEXT NOT NULL,position INTEGER NOT NULL,
        PRIMARY KEY(identity,namespace,profile,generation));",
    )?;
    Ok(())
}

/// Identifies one enrolled copy: the catalog observation that supplies records
/// and the local history that receives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyIdentity {
    pub profile: Uuid,
    pub generation: Uuid,
    pub source: Uuid,
    pub device: Uuid,
}
impl CopyIdentity {
    fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.profile.is_nil()
                && !self.generation.is_nil()
                && !self.source.is_nil()
                && !self.device.is_nil(),
            "The profile copy identity is incomplete."
        );
        Ok(())
    }
}

impl Journal {
    /// The last exported catalog position for this exact source/device pair.
    /// Any other pair starts at zero so a rebuilt journal cannot skip records.
    pub async fn copy_position(
        &self,
        binding: &Binding,
        identity: CopyIdentity,
    ) -> anyhow::Result<u64> {
        binding.validate()?;
        identity.validate()?;
        let binding = binding.clone();
        self.worker
            .run(move |c| {
                let row: Option<(String, String, i64)> = c
                    .query_row(
                        "SELECT source,device,position FROM catalog_copies WHERE identity=? AND namespace=? AND profile=? AND generation=?",
                        params![binding.identity, binding.namespace, identity.profile.to_string(), identity.generation.to_string()],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                match row {
                    Some((source, device, position))
                        if source == identity.source.to_string()
                            && device == identity.device.to_string() =>
                    {
                        Ok(u64::try_from(position)?)
                    }
                    _ => Ok(0),
                }
            })
            .await
    }

    /// Save only after the record's history import committed. A lost save is
    /// repaired by re-exporting the same immutable record.
    pub async fn checkpoint_copy(
        &self,
        binding: &Binding,
        identity: CopyIdentity,
        position: u64,
    ) -> anyhow::Result<()> {
        binding.validate()?;
        identity.validate()?;
        let binding = binding.clone();
        let position = i64::try_from(position)?;
        self.worker
            .run(move |c| {
                c.execute(
                    "INSERT INTO catalog_copies(identity,namespace,profile,generation,source,device,position) VALUES(?,?,?,?,?,?,?)
                    ON CONFLICT(identity,namespace,profile,generation) DO UPDATE SET source=excluded.source,device=excluded.device,position=excluded.position",
                    params![binding.identity, binding.namespace, identity.profile.to_string(), identity.generation.to_string(), identity.source.to_string(), identity.device.to_string(), position],
                )?;
                Ok(())
            })
            .await
    }
}
