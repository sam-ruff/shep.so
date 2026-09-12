//! Bounded physical lineage for pages overtaking a recovered move's receipt.
use super::*;

fn same(a: &Mail, b: &Mail) -> bool {
    a.id == b.id
        && a.account_id == b.account_id
        && a.folder == b.folder
        && a.remote_id == b.remote_id
}

fn linked(previous: &MoveRecord, next: &MoveRecord) -> bool {
    previous.stage == MoveStage::Located
        && next.stage == MoveStage::Located
        && previous.receipt.account == previous.original.account_id
        && previous
            .receipt
            .current
            .as_ref()
            .is_some_and(|mail| same(mail, &next.original))
        && previous.receipt.fingerprint.is_some()
        && previous.receipt.fingerprint == next.receipt.fingerprint
        && previous
            .receipt
            .connections
            .iter()
            .all(|connection| next.receipt.connections.contains(connection))
}

fn completed_for_source(c: &Connection, id: &str) -> anyhow::Result<Option<MoveRecord>> {
    let data: Option<String> = c
        .query_row(
            "SELECT data FROM mail_moves WHERE source_id=? AND stage IN ('located','kept')",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(data) = data else {
        return Ok(None);
    };
    let record: MoveRecord = serde_json::from_str(&data)?;
    anyhow::ensure!(
        record.original.id == id && record.finished(),
        "The recovered mailbox identity is inconsistent."
    );
    record.validate_receipt(&record.receipt)?;
    Ok(Some(record))
}

fn cached(c: &Connection, expected: &Mail) -> anyhow::Result<Option<Mail>> {
    let data: Option<(String, String, String, bool, bool)> = c
        .query_row(
            "SELECT data,account,folder,unread,starred FROM messages WHERE id=?",
            [&expected.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((data, account, folder, unread, starred)) = data else {
        return Ok(None);
    };
    let mut mail: Mail = serde_json::from_str(&data)?;
    anyhow::ensure!(
        same(&mail, expected) && mail.account_id == account && mail.folder == folder,
        "The recovered message's cached identity changed."
    );
    mail.unread = unread;
    mail.starred = starred;
    Ok(Some(mail))
}

pub(in crate::store) fn observed(c: &Connection, id: &str) -> anyhow::Result<Option<Mail>> {
    let Some(previous) = completed_for_source(c, id)? else {
        return Ok(None);
    };
    let Some(intermediate) = previous.resolved_mail() else {
        return Ok(None);
    };
    if let Some(mail) = cached(c, intermediate)? {
        return Ok(Some(mail));
    }
    let Some(next) = completed_for_source(c, &intermediate.id)? else {
        return Ok(None);
    };
    if !linked(&previous, &next) {
        return Ok(None);
    }
    let Some(current) = next.resolved_mail() else {
        return Ok(None);
    };
    cached(c, current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_actions::Fingerprint;

    #[test]
    fn successor_requires_physical_identity_fingerprint_and_connection_continuity() {
        let mail = crate::model::parse_mail(
            "work",
            "42.1",
            "Projects",
            b"Subject: chain\r\n\r\nExact body".to_vec(),
            true,
            false,
        )
        .expect("mail");
        let fingerprint = Fingerprint::of(&mail.raw);
        let mut first = MoveRecord::new(
            mail.summary.clone(),
            MoveReceipt::server(
                &mail.summary,
                "work",
                "INBOX",
                Some("43.2".into()),
                fingerprint.clone(),
            ),
        );
        first.stage = MoveStage::Located;
        first.receipt.connections = vec![("work".into(), "server-one".into())];
        let intermediate = first.receipt.current.clone().expect("intermediate");
        let mut next = MoveRecord::new(
            intermediate.clone(),
            MoveReceipt::server(
                &intermediate,
                "work",
                "Trash",
                Some("44.3".into()),
                fingerprint,
            ),
        );
        next.stage = MoveStage::Located;
        next.receipt.connections = first.receipt.connections.clone();
        assert!(linked(&first, &next));
        for change in [
            "uid",
            "folder",
            "account",
            "fingerprint",
            "connection",
            "pending",
        ] {
            let mut changed = next.clone();
            match change {
                "uid" => changed.original.remote_id = "43.99".into(),
                "folder" => changed.original.folder = "Other".into(),
                "account" => changed.original.account_id = "other".into(),
                "fingerprint" => {
                    changed.receipt.fingerprint = Some(Fingerprint::of(b"Different bytes"))
                }
                "connection" => changed.receipt.connections[0].1 = "server-two".into(),
                "pending" => changed.stage = MoveStage::Committed,
                _ => {}
            }
            assert!(!linked(&first, &changed), "{change}");
        }
    }
}
