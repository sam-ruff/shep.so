//! Runs only on a validated private copy, before it can be selected as a local
//! profile. Preserve original operation metadata; never replay another device's
//! pending work merely because it was pending at the export's read point.
use super::*;
use crate::{
    outgoing::{DeliveryState, OutgoingInfo, SentState},
    profiles::{IMPORT_MARKER_KEY, ImportMarker},
};
use rusqlite::params;

const REVIEW_NOTE: &str = "Imported from another device. This action may have finished there after export. Check the server before resolving it.";

pub(super) fn apply(
    path: &Path,
    id: uuid::Uuid,
    name: &str,
    local: &Preferences,
    cancel: &watch::Receiver<bool>,
) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    let mut c = Connection::open(path)?;
    defensive(&c, cancel)?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let tx = c.transaction()?;
    // The copy was validated as v2/v3; bring its archive table/version forward
    // in the same transaction as the device-specific import preparation.
    crate::store::import_archive_schema(&tx)?;
    let import_id = id.to_string();
    let already: Option<String> = tx
        .query_row(
            "SELECT value FROM kv WHERE key=?",
            [IMPORT_MARKER_KEY],
            |r| r.get(0),
        )
        .optional()?;
    if already.as_ref().is_some_and(|value| {
        serde_json::from_str::<ImportMarker>(value)
            .is_ok_and(|marker| marker.version == 1 && marker.local_profile == id)
    }) {
        return Ok(());
    }

    tx.execute("INSERT INTO imported_operations SELECT ?,'preferences',key,value FROM kv WHERE key='preferences'", [&import_id])?;
    tx.execute(
        "INSERT INTO imported_operations SELECT ?,'profile-marker',key,value FROM kv WHERE key=?",
        params![import_id, IMPORT_MARKER_KEY],
    )?;
    // No original MIME is duplicated: journals continue to refer to the same
    // message/outgoing attachment rows. Archive only metadata being changed.
    tx.execute("INSERT INTO imported_operations
        SELECT ?,'bulk-job',id,json_object('paused',paused)
        FROM bulk_jobs WHERE EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id AND status NOT IN ('done','cancelled'))", [&import_id])?;
    tx.execute("INSERT INTO imported_operations
        SELECT ?,'bulk-item',json_array(job,position),json_object('status',status,'undo',undo,'receipt',receipt,'error',error)
        FROM bulk_items WHERE status NOT IN ('done','cancelled')", [&import_id])?;
    tx.execute("INSERT INTO imported_operations
        SELECT ?,'bulk-effect',id,json_object('job',job,'position',position,'account',account,'folder',folder,'unread',unread,'starred',starred)
        FROM bulk_effects", [&import_id])?;
    tx.execute("UPDATE bulk_jobs SET paused=1 WHERE EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id AND status NOT IN ('done','cancelled'))", [])?;
    tx.execute(
        "UPDATE bulk_items SET status='uncertain',error=? WHERE status NOT IN ('done','cancelled')",
        [REVIEW_NOTE],
    )?;
    tx.execute_batch("DELETE FROM bulk_effects;
        DELETE FROM bulk_totals;
        INSERT INTO bulk_totals SELECT job,status,undo,count(*) FROM bulk_items GROUP BY job,status,undo;")?;

    let uncertain = serde_json::to_string(&crate::folder_actions::Status::Uncertain)?;
    let statuses = [
        crate::folder_actions::Status::Queued,
        crate::folder_actions::Status::Running,
        crate::folder_actions::Status::Rejected,
    ];
    for status in statuses {
        let status = serde_json::to_string(&status)?;
        tx.execute("INSERT INTO imported_operations
            SELECT ?,'folder-step',json_array(job,position),json_object('status',status,'error',error)
            FROM folder_steps WHERE status=?", params![import_id,status])?;
        tx.execute(
            "UPDATE folder_steps SET status=?,error=? WHERE status=?",
            params![uncertain, REVIEW_NOTE, status],
        )?;
    }
    // Acknowledged folder steps stay acknowledged: their cache-only completion
    // may proceed, but no unacknowledged step is runnable after this import.

    let mut after = String::new();
    loop {
        check_cancel(cancel)?;
        // One metadata record at a time, independent of group/mailbox size.
        let next: Option<(String, String, String)> = tx
            .query_row(
                "SELECT attempt,stage,data FROM outgoing WHERE attempt>? ORDER BY attempt LIMIT 1",
                [&after],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((attempt, stage, data)) = next else {
            break;
        };
        after = attempt.clone();
        let mut info: OutgoingInfo = serde_json::from_str(&data).context(
            "An outgoing record is unreadable. Export a fresh copy after reviewing Outbox.",
        )?;
        anyhow::ensure!(
            format!("{:?}", info.delivery) == stage && info.attempt == attempt,
            "An outgoing record has inconsistent delivery state. Review Outbox on the original device."
        );
        let mut changed = false;
        if matches!(
            info.delivery,
            DeliveryState::Submitting | DeliveryState::Uncertain | DeliveryState::Rejected
        ) {
            info.delivery = DeliveryState::Uncertain;
            changed = true;
        }
        if info.delivery == DeliveryState::Accepted
            && matches!(
                info.sent,
                SentState::Pending | SentState::Appending | SentState::Uncertain
            )
        {
            info.sent = SentState::Uncertain;
            changed = true;
        }
        if changed {
            tx.execute(
                "INSERT INTO imported_operations VALUES(?,'outgoing',?,?)",
                params![import_id, attempt, data],
            )?;
            info.error = Some(REVIEW_NOTE.into());
            tx.execute(
                "UPDATE outgoing SET stage=?,data=? WHERE attempt=?",
                params![
                    format!("{:?}", info.delivery),
                    serde_json::to_string(&info)?,
                    attempt
                ],
            )?;
        }
    }

    tx.execute("INSERT INTO imported_operations
        SELECT ?,'credential-cleanup',json_array(kind,id,key),json_object('kind',kind,'id',id,'key',key)
        FROM credential_cleanup", [&import_id])?;
    tx.execute("DELETE FROM credential_cleanup", [])?;
    // These observations belong to the other device. Its original mailbox
    // contents remain, and the first successful sync here establishes a quiet
    // notification baseline rather than announcing an old mailbox as new mail.
    tx.execute_batch("DELETE FROM notification_seen; DELETE FROM notification_mailboxes;")?;

    let mut preferences: Preferences = setting(&tx, "preferences")?;
    preferences.window_size = local.window_size;
    preferences.backup_folder = local.backup_folder.clone();
    preferences.auto_backup = false;
    preferences.backup_ready = false;
    preferences.last_backup = None;
    preferences.google_connection_id.clear();
    preferences.google_grant = Default::default();
    preferences.google_lifecycle = crate::model::GoogleLifecycle {
        disconnected: true,
        ..Default::default()
    };
    // Enrollment and its external history belong to the source device. Archive
    // the local pointer for review, then require fresh discovery on this copy.
    for key in [
        crate::profile_sync::enrollment::STORAGE_KEY,
        crate::profile_sync::enrollment::SEED_KEY,
    ] {
        tx.execute("INSERT INTO imported_operations SELECT ?,'profile-enrollment',key,value FROM kv WHERE key=?",params![import_id,key])?;
        tx.execute("DELETE FROM kv WHERE key=?", [key])?;
    }
    tx.execute("INSERT INTO kv(key,value) VALUES('preferences',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&preferences)?])?;
    let marker = ImportMarker {
        version: 1,
        local_profile: id,
        name: name.into(),
    };
    tx.execute(
        "INSERT INTO kv(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![IMPORT_MARKER_KEY, serde_json::to_string(&marker)?],
    )?;
    tx.pragma_update(None, "user_version", crate::store::DATABASE_VERSION)?;
    check_cancel(cancel)?;
    tx.commit()?;
    c.close().map_err(|(_, error)| error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
