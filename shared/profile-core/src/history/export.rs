use super::*;
use rusqlite::OptionalExtension;

impl Journal {
    /// Read a stable history one bounded record at a time. The caller captures
    /// the initialized revision first; any intervening change rejects the copy.
    pub fn export_record(&self, expected_revision: u64, after: u64) -> Result<Option<Record>> {
        self.export_kind(expected_revision, after, false)
    }
    /// Inspect known remote ancestry one original at a time, without scanning
    /// over a potentially large queue of unpublished local operations.
    pub fn export_acknowledged_record(
        &self,
        expected_revision: u64,
        after: u64,
    ) -> Result<Option<Record>> {
        self.export_kind(expected_revision, after, true)
    }
    fn export_kind(
        &self,
        expected_revision: u64,
        after: u64,
        acknowledged: bool,
    ) -> Result<Option<Record>> {
        let state = self.state()?;
        if state.revision != expected_revision {
            return Err(Error::Changed);
        }
        if !state.initialized {
            return Err(Error::Incomplete);
        }
        let after = i64::try_from(after).map_err(|_| Error::Changed)?;
        self.db
            .query_row(
                if acknowledged {
                    "SELECT seq,id,raw FROM operations WHERE seq>? AND (local=0 OR uploaded=1) ORDER BY seq LIMIT 1"
                } else {
                    "SELECT seq,id,raw FROM operations WHERE seq>? ORDER BY seq LIMIT 1"
                },
                [after],
                |r| {
                    Ok((
                        count(r, 0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?
            .map(|(position, operation, raw)| {
                if raw.len() > crate::MAX_RECORD_BYTES {
                    return Err(Error::Storage);
                }
                Ok(Record {
                    position,
                    operation: parse_uuid(&operation)?,
                    record: String::from_utf8(raw).map_err(|_| Error::Storage)?,
                })
            })
            .transpose()
    }
}
