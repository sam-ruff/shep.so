//! Content-free ledger of acknowledged local mail writes. A check lists a
//! folder at SELECT/SEARCH time and applies the result later, so a write that
//! landed after that listing must win when the stale item arrives. Entries hold
//! ids and kinds only and live in the session scratch database.
use super::*;

/// Entries kept per account; the oldest are dropped first.
pub(crate) const LIMIT_PER_ACCOUNT: usize = 256;
/// A check never outlives its cycle bound, so an acknowledgement older than
/// this cannot be undone by any listing still being applied.
pub(crate) const GRACE_SECONDS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    /// Read or flag state written after the server confirmed it.
    Flags,
    /// The source identity of an acknowledged move; the row is gone.
    MovedAway,
    /// The destination identity of an acknowledged move; the row is new.
    MovedIn,
}
impl WriteKind {
    fn key(self) -> &'static str {
        match self {
            WriteKind::Flags => "flags",
            WriteKind::MovedAway => "moved-away",
            WriteKind::MovedIn => "moved-in",
        }
    }
    fn parse(key: &str) -> anyhow::Result<Self> {
        Ok(match key {
            "flags" => WriteKind::Flags,
            "moved-away" => WriteKind::MovedAway,
            "moved-in" => WriteKind::MovedIn,
            other => anyhow::bail!("Unknown write ledger kind {other}"),
        })
    }
}

/// The ledger clock value taken when a check began, before any listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SyncEpoch(i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub kind: WriteKind,
    pub started: i64,
    pub acknowledged: Option<i64>,
}
impl Entry {
    /// Whether this write outranks items from a check that began at `epoch`.
    /// Every listing of that check was taken at or after the epoch, so only a
    /// write acknowledged before the check began is already reflected there.
    pub fn outranks(&self, epoch: SyncEpoch) -> bool {
        self.acknowledged
            .is_none_or(|acknowledged| acknowledged > epoch.0)
    }
    /// Whether a check that began at `epoch` observed the server after this
    /// write was acknowledged, so finishing that check clears the entry.
    pub fn observed_by(&self, epoch: SyncEpoch) -> bool {
        self.acknowledged
            .is_some_and(|acknowledged| acknowledged < epoch.0)
    }
}

/// Decisions for one message id against the check that produced an item.
pub fn outranked(entries: &[Entry], kind: WriteKind, epoch: SyncEpoch) -> bool {
    entries
        .iter()
        .any(|entry| entry.kind == kind && entry.outranks(epoch))
}
/// A server flag listing keeps the local flags of a ledgered flag write.
pub fn keeps_local_flags(entries: &[Entry], epoch: SyncEpoch) -> bool {
    outranked(entries, WriteKind::Flags, epoch)
}
/// A body arrival for an id the user moved away is dropped.
pub fn drops_arrival(entries: &[Entry], epoch: SyncEpoch) -> bool {
    outranked(entries, WriteKind::MovedAway, epoch)
}
/// A listing that predates a move must not remove the destination copy.
pub fn keeps_row(entries: &[Entry], epoch: SyncEpoch) -> bool {
    outranked(entries, WriteKind::MovedIn, epoch)
}
/// A folder's CONDSTORE state from a check waits while any write to one of
/// its messages (ids starting with `prefix`) outranks that check.
pub fn holds_folder_state(entries: &[Entry], prefix: &str, epoch: SyncEpoch) -> bool {
    entries
        .iter()
        .any(|entry| entry.id.starts_with(prefix) && entry.outranks(epoch))
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE scratch.write_clock(value INTEGER NOT NULL);
        INSERT INTO scratch.write_clock VALUES(0);
        CREATE TABLE scratch.write_ledger(
            sequence INTEGER PRIMARY KEY, account TEXT NOT NULL, id TEXT NOT NULL,
            kind TEXT NOT NULL, acknowledged INTEGER, acknowledged_at INTEGER);
        CREATE INDEX scratch.write_ledger_account ON write_ledger(account,sequence);
        CREATE INDEX scratch.write_ledger_id ON write_ledger(id);",
    )?;
    Ok(())
}

fn tick(c: &Connection) -> anyhow::Result<i64> {
    c.execute("UPDATE scratch.write_clock SET value=value+1", [])?;
    Ok(c.query_row("SELECT value FROM scratch.write_clock", [], |r| r.get(0))?)
}

/// Record a write the server has acknowledged, in the same transaction as
/// its cache effect. Returns the entry's sequence.
pub(super) fn record_acknowledged(
    c: &Connection,
    account: &str,
    id: &str,
    kind: WriteKind,
    now: i64,
) -> anyhow::Result<i64> {
    let sequence = tick(c)?;
    c.execute(
        "INSERT INTO scratch.write_ledger(sequence,account,id,kind,acknowledged,acknowledged_at) VALUES(?,?,?,?,?,?)",
        params![sequence, account, id, kind.key(), sequence, now],
    )?;
    bound(c, account)?;
    Ok(sequence)
}

fn bound(c: &Connection, account: &str) -> anyhow::Result<()> {
    c.execute(
        "DELETE FROM scratch.write_ledger WHERE account=?1 AND sequence NOT IN (
            SELECT sequence FROM scratch.write_ledger WHERE account=?1 ORDER BY sequence DESC LIMIT ?2)",
        params![account, LIMIT_PER_ACCOUNT as i64],
    )?;
    Ok(())
}

pub(super) fn entries_for(c: &Connection, id: &str) -> anyhow::Result<Vec<Entry>> {
    c.prepare_cached(
        "SELECT id,kind,sequence,acknowledged FROM scratch.write_ledger WHERE id=? ORDER BY sequence",
    )?
    .query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<i64>>(3)?,
        ))
    })?
    .map(|row| {
        let (id, kind, started, acknowledged) = row?;
        Ok(Entry {
            id,
            kind: WriteKind::parse(&kind)?,
            started,
            acknowledged,
        })
    })
    .collect()
}

pub(super) fn entries(c: &Connection, account: &str) -> anyhow::Result<Vec<Entry>> {
    c.prepare(
        "SELECT id,kind,sequence,acknowledged FROM scratch.write_ledger WHERE account=? ORDER BY sequence",
    )?
    .query_map([account], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<i64>>(3)?,
        ))
    })?
    .map(|row| {
        let (id, kind, started, acknowledged) = row?;
        Ok(Entry {
            id,
            kind: WriteKind::parse(&kind)?,
            started,
            acknowledged,
        })
    })
    .collect()
}

/// Take the epoch for a check that is about to list folders, and drop entries
/// no running check can still undo.
pub(super) fn begin_check(c: &Connection, now: i64) -> anyhow::Result<SyncEpoch> {
    c.execute(
        "DELETE FROM scratch.write_ledger WHERE acknowledged_at IS NOT NULL AND acknowledged_at<?",
        [now - GRACE_SECONDS],
    )?;
    Ok(SyncEpoch(tick(c)?))
}

/// A finished check observed every write acknowledged before it began.
pub(super) fn finish_check(c: &Connection, account: &str, epoch: SyncEpoch) -> anyhow::Result<()> {
    c.execute(
        "DELETE FROM scratch.write_ledger WHERE account=? AND acknowledged IS NOT NULL AND acknowledged<?",
        params![account, epoch.0],
    )?;
    Ok(())
}

impl Store {
    /// Call before a check's first SELECT so every listing it takes is at or
    /// after the epoch.
    pub async fn sync_epoch(&self) -> anyhow::Result<SyncEpoch> {
        let now = chrono::Utc::now().timestamp();
        self.run(move |c| begin_check(c, now)).await
    }
    /// Clears the account's writes that a completed check observed.
    pub async fn sync_finished(&self, account: String, epoch: SyncEpoch) -> anyhow::Result<()> {
        self.run(move |c| finish_check(c, &account, epoch)).await
    }
    pub async fn write_ledger(&self, account: String) -> anyhow::Result<Vec<Entry>> {
        self.run(move |c| entries(c, &account)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: WriteKind, acknowledged: Option<i64>) -> Entry {
        Entry {
            id: "work:INBOX:1.7".into(),
            kind,
            started: acknowledged.unwrap_or(5),
            acknowledged,
        }
    }

    #[test]
    fn a_write_acknowledged_after_the_check_began_outranks_its_listing() {
        let epoch = SyncEpoch(10);
        assert!(entry(WriteKind::Flags, Some(11)).outranks(epoch));
        assert!(entry(WriteKind::Flags, None).outranks(epoch));
        assert!(!entry(WriteKind::Flags, Some(9)).outranks(epoch));
        assert!(!entry(WriteKind::Flags, Some(10)).outranks(epoch));
    }

    #[test]
    fn only_a_check_started_after_the_acknowledgement_observes_it() {
        assert!(entry(WriteKind::Flags, Some(9)).observed_by(SyncEpoch(10)));
        assert!(!entry(WriteKind::Flags, Some(10)).observed_by(SyncEpoch(10)));
        assert!(!entry(WriteKind::Flags, Some(11)).observed_by(SyncEpoch(10)));
        assert!(!entry(WriteKind::Flags, None).observed_by(SyncEpoch(10)));
    }

    #[test]
    fn only_an_outranking_write_in_the_folder_holds_its_state() {
        let epoch = SyncEpoch(10);
        let inbox = "work:INBOX:";
        let later = [entry(WriteKind::Flags, Some(11))];
        assert!(holds_folder_state(&later, inbox, epoch));
        assert!(holds_folder_state(
            &[entry(WriteKind::MovedAway, None)],
            inbox,
            epoch
        ));
        assert!(!holds_folder_state(
            &[entry(WriteKind::Flags, Some(9))],
            inbox,
            epoch
        ));
        assert!(!holds_folder_state(&later, "work:Archive:", epoch));
    }

    #[test]
    fn decisions_match_their_write_kind() {
        let epoch = SyncEpoch(10);
        let flags = [entry(WriteKind::Flags, Some(12))];
        let away = [entry(WriteKind::MovedAway, Some(12))];
        let moved_in = [entry(WriteKind::MovedIn, Some(12))];
        assert!(keeps_local_flags(&flags, epoch));
        assert!(!keeps_local_flags(&away, epoch));
        assert!(drops_arrival(&away, epoch));
        assert!(!drops_arrival(&moved_in, epoch));
        assert!(keeps_row(&moved_in, epoch));
        assert!(!keeps_row(&flags, epoch));
        assert!(!keeps_row(&[], epoch));
        assert!(!keeps_local_flags(
            &[entry(WriteKind::Flags, Some(3))],
            epoch
        ));
    }

    #[tokio::test]
    async fn ledger_records_clears_after_a_later_check_and_stays_bounded() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let stale = store.sync_epoch().await?;
        let now = chrono::Utc::now().timestamp();
        store
            .run(move |c| {
                record_acknowledged(c, "work", "work:INBOX:1.7", WriteKind::Flags, now)?;
                Ok(())
            })
            .await?;
        let entries = store.write_ledger("work".into()).await?;
        assert_eq!(entries.len(), 1);
        assert!(keeps_local_flags(&entries, stale));
        // The stale check finishing does not clear a write it never observed.
        store.sync_finished("work".into(), stale).await?;
        assert_eq!(store.write_ledger("work".into()).await?.len(), 1);
        let later = store.sync_epoch().await?;
        assert!(!keeps_local_flags(
            &store.write_ledger("work".into()).await?,
            later
        ));
        store.sync_finished("other".into(), later).await?;
        assert_eq!(store.write_ledger("work".into()).await?.len(), 1);
        store.sync_finished("work".into(), later).await?;
        assert!(store.write_ledger("work".into()).await?.is_empty());
        store
            .run(move |c| {
                for index in 0..LIMIT_PER_ACCOUNT + 10 {
                    let id = format!("work:INBOX:1.{index}");
                    record_acknowledged(c, "work", &id, WriteKind::MovedIn, now)?;
                }
                record_acknowledged(c, "other", "other:INBOX:1.1", WriteKind::MovedIn, now)?;
                Ok(())
            })
            .await?;
        let entries = store.write_ledger("work".into()).await?;
        assert_eq!(entries.len(), LIMIT_PER_ACCOUNT);
        assert_eq!(
            entries[0].id, "work:INBOX:1.10",
            "the oldest entries go first"
        );
        assert_eq!(store.write_ledger("other".into()).await?.len(), 1);
        Ok(())
    }

    fn fixture(remote: &str, folder: &str) -> StoredMail {
        parse_mail(
            "work",
            remote,
            folder,
            format!("Message-ID: <{remote}@example.test>\r\nSubject: Ledger\r\n\r\nBody")
                .into_bytes(),
            true,
            false,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn stale_server_flags_keep_a_flag_written_after_the_check_began() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let mail = fixture("1.7", "INBOX");
        let summary = mail.summary.clone();
        store.upsert(vec![mail]).await?;
        let stale = store.sync_epoch().await?;
        store
            .patch_flags(
                summary.clone(),
                crate::mail_actions::Flags {
                    unread: None,
                    starred: Some(true),
                },
            )
            .await?;
        let listing = || MailSyncItem::Flags(vec![(summary.id.clone(), true, false)]);
        store.apply_sync_since(listing(), Some(stale)).await?;
        assert!(store.mail_metadata(summary.id.clone()).await?.starred);
        store.sync_finished("work".into(), stale).await?;
        assert_eq!(store.write_ledger("work".into()).await?.len(), 1);
        // A check that began after the acknowledgement sees the server as is,
        // so a change made on another client wins from then on.
        let later = store.sync_epoch().await?;
        store.apply_sync_since(listing(), Some(later)).await?;
        assert!(!store.mail_metadata(summary.id.clone()).await?.starred);
        store.sync_finished("work".into(), later).await?;
        assert!(store.write_ledger("work".into()).await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_listing_taken_before_a_move_keeps_the_destination_and_drops_the_source()
    -> anyhow::Result<()> {
        use crate::mail_actions::MoveReceipt;
        for destination in ["Archive", "Deleted Items"] {
            let store = Store::memory()?;
            let mail = fixture("1.7", "INBOX");
            let source = mail.summary.clone();
            store.upsert(vec![mail.clone()]).await?;
            let stale = store.sync_epoch().await?;
            let fingerprint = store.message_fingerprint(source.id.clone()).await?;
            let moved = MoveReceipt::server(
                &source,
                "work",
                destination,
                Some("2.9".into()),
                fingerprint,
            )
            .current
            .context("server receipts carry the new identity")?;
            store.relocate_mail(source.clone(), moved.clone()).await?;
            let listing = |folder: &str| MailSyncItem::Reconcile {
                account: "work".into(),
                folder: folder.into(),
                live_ids: std::collections::HashSet::from([source.id.clone()]),
            };
            store
                .apply_sync_since(listing(destination), Some(stale))
                .await?;
            assert!(
                store.mail_metadata(moved.id.clone()).await.is_ok(),
                "destination copy kept"
            );
            store
                .apply_sync_since(MailSyncItem::Message(mail.clone()), Some(stale))
                .await?;
            assert!(
                store
                    .sync_message_since(mail.clone(), Some(stale))
                    .await?
                    .is_none()
            );
            assert!(
                store.mail_metadata(source.id.clone()).await.is_err(),
                "source not restored"
            );
            store
                .apply_sync_since(listing("INBOX"), Some(stale))
                .await?;
            assert!(store.mail_metadata(moved.id.clone()).await.is_ok());
            store.sync_finished("work".into(), stale).await?;
            let later = store.sync_epoch().await?;
            store
                .apply_sync_since(listing(destination), Some(later))
                .await?;
            assert!(
                store.mail_metadata(moved.id.clone()).await.is_err(),
                "later listings rule"
            );
            store
                .apply_sync_since(MailSyncItem::Message(mail.clone()), Some(later))
                .await?;
            assert!(store.mail_metadata(source.id.clone()).await.is_ok());
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_staged_body_for_a_moved_away_id_is_dropped() -> anyhow::Result<()> {
        use crate::mail_actions::MoveReceipt;
        use std::io::Write;
        let store = Store::memory()?;
        let mail = fixture("1.7", "INBOX");
        let source = mail.summary.clone();
        store.upsert(vec![mail]).await?;
        let stale = store.sync_epoch().await?;
        let fingerprint = store.message_fingerprint(source.id.clone()).await?;
        let moved =
            MoveReceipt::server(&source, "work", "Archive", Some("2.9".into()), fingerprint)
                .current
                .context("server receipts carry the new identity")?;
        store.relocate_mail(source.clone(), moved).await?;
        let staged = || {
            let mut file = tempfile::NamedTempFile::new()?;
            file.write_all(b"Subject: Staged\r\n\r\nBody")?;
            shep_mail_core::providers::mail::staging::prepare(
                file, "work", "1.7", "INBOX", true, false,
            )
        };
        assert!(
            store
                .sync_staged_message_since(staged()?, Some(stale))
                .await?
                .is_none()
        );
        assert!(store.mail_metadata(source.id.clone()).await.is_err());
        store.sync_finished("work".into(), stale).await?;
        let later = store.sync_epoch().await?;
        store
            .sync_staged_message_since(staged()?, Some(later))
            .await?;
        assert!(store.mail_metadata(source.id).await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn entries_older_than_the_cycle_bound_are_pruned_when_a_check_begins()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        store
            .run(|c| {
                record_acknowledged(c, "work", "work:INBOX:1.1", WriteKind::Flags, 1_000)?;
                record_acknowledged(c, "work", "work:INBOX:1.2", WriteKind::Flags, 2_000)?;
                begin_check(c, 1_000 + GRACE_SECONDS + 1)?;
                Ok(())
            })
            .await?;
        let entries = store.write_ledger("work".into()).await?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "work:INBOX:1.2");
        Ok(())
    }
}
