//! QRESYNC (RFC 7162) expunge discovery. A folder with a usable saved state
//! learns expunged UIDs from VANISHED instead of listing every UID; anything
//! that does not account for the mailbox size uses the complete listing.
use super::*;
use async_imap::imap_proto::{Capability, MailboxDatum, Response, Status};
use std::collections::{BTreeSet, HashMap};
use std::ops::RangeInclusive;

/// Most VANISHED ranges accepted for one folder; a longer reply uses the
/// complete listing instead.
pub(super) const MAX_RANGES: usize = 100_000;

/// Sorted, merged UID ranges with binary-search membership.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Ranges(Vec<(u32, u32)>);

impl Ranges {
    pub(super) fn merged(mut ranges: Vec<(u32, u32)>) -> Self {
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            match merged.last_mut() {
                Some(last) if start <= last.1.saturating_add(1) => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        Self(merged)
    }

    pub(super) fn contains(&self, uid: u32) -> bool {
        let index = self.0.partition_point(|(start, _)| *start <= uid);
        index > 0 && self.0[index - 1].1 >= uid
    }
}

/// Cached UIDs per folder and UIDVALIDITY, built once per check from the
/// cached message identities `{account}:{folder}:{validity}.{uid}`.
#[derive(Debug, Default)]
pub(super) struct Index(HashMap<String, HashMap<u32, HashSet<u32>>>);

impl Index {
    pub(super) fn new(account: &str, known: &HashSet<String>) -> Self {
        let prefix = format!("{account}:");
        let mut index: HashMap<String, HashMap<u32, HashSet<u32>>> = HashMap::new();
        for id in known {
            let Some(rest) = id.strip_prefix(&prefix) else {
                continue;
            };
            let Some((folder, position)) = rest.rsplit_once(':') else {
                continue;
            };
            let Some((validity, uid)) = position.split_once('.') else {
                continue;
            };
            let (Ok(validity), Ok(uid)) = (validity.parse::<u32>(), uid.parse::<u32>()) else {
                continue;
            };
            index
                .entry(folder.to_owned())
                .or_default()
                .entry(validity)
                .or_default()
                .insert(uid);
        }
        Self(index)
    }

    pub(super) fn cached(&self, folder: &str, validity: u32) -> HashSet<u32> {
        self.0
            .get(folder)
            .and_then(|folder| folder.get(&validity))
            .cloned()
            .unwrap_or_default()
    }
}

/// What a QRESYNC refresh changed in a folder's cache.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Outcome {
    /// Cached UIDs the server expunged.
    pub vanished: Vec<u32>,
    /// Uncached UIDs to download, newest first.
    pub new: Vec<u32>,
}

/// Applies a VANISHED reply to the cached UIDs. `None` when the result does
/// not add up to the SELECT size, for example a message whose body never
/// downloaded, so only the complete listing can reconcile the folder.
pub(super) fn reconcile(
    cached: &HashSet<u32>,
    vanished: &Ranges,
    reported: impl IntoIterator<Item = u32>,
    exists: u32,
) -> Option<Outcome> {
    let mut gone: Vec<u32> = cached
        .iter()
        .copied()
        .filter(|uid| vanished.contains(*uid))
        .collect();
    gone.sort_unstable();
    let new: BTreeSet<u32> = reported
        .into_iter()
        .filter(|uid| !cached.contains(uid) && !vanished.contains(*uid))
        .collect();
    let size = cached.len() - gone.len() + new.len();
    (u32::try_from(size).ok() == Some(exists)).then(|| Outcome {
        vanished: gone,
        new: new.into_iter().rev().collect(),
    })
}

/// Sends `ENABLE QRESYNC`. `Ok(false)` when the server refused it, which
/// leaves the session usable without VANISHED.
pub(super) async fn enable<T>(session: &mut async_imap::Session<T>) -> anyhow::Result<bool>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session.run_command("ENABLE QRESYNC").await?;
    let mut enabled = false;
    loop {
        let response = session
            .read_response()
            .await?
            .context("The mail server disconnected before completing sync. Try Refresh again.")?;
        match response.parsed() {
            Response::Capabilities(capabilities) => {
                enabled |= capabilities.iter().any(|capability| {
                    matches!(capability, Capability::Atom(name) if name.eq_ignore_ascii_case("QRESYNC"))
                });
            }
            Response::Done {
                tag: actual,
                status,
                ..
            } if *actual == tag => return Ok(enabled && *status == Status::Ok),
            Response::Data {
                status: Status::Bye,
                ..
            } => anyhow::bail!("The mail server disconnected during sync. Try Refresh again."),
            _ => {}
        }
    }
}

/// The reply to a CHANGEDSINCE fetch with VANISHED.
pub(super) struct Changes {
    pub fetched: Vec<sync_queries::Fetch>,
    pub vanished: Ranges,
    /// The mailbox changed while the reply was sent (live VANISHED, EXPUNGE
    /// or EXISTS), so the SELECT size no longer describes it.
    pub disturbed: bool,
}

/// Flags changed after `since` plus UIDs expunged since then. A rejected
/// request is an error, so partial VANISHED data before NO is never used.
pub(super) async fn changes<T>(
    session: &mut async_imap::Session<T>,
    since: u64,
) -> anyhow::Result<Changes>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session
        .run_command(format!(
            "UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE {since} VANISHED)"
        ))
        .await?;
    let mut fetched = Vec::new();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut disturbed = false;
    loop {
        let response = session
            .read_response()
            .await?
            .context("The mail server disconnected before completing sync. Try Refresh again.")?;
        match response.parsed() {
            Response::Fetch(_, attributes) => fetched.push(sync_queries::parse_fetch(attributes)?),
            Response::Vanished {
                earlier: true,
                uids,
            } => {
                anyhow::ensure!(
                    ranges.len() + uids.len() <= MAX_RANGES,
                    "The mail server reported too many removed messages."
                );
                ranges.extend(uids.iter().map(bounds));
            }
            Response::Vanished { earlier: false, .. }
            | Response::Expunge(_)
            | Response::MailboxData(MailboxDatum::Exists(_)) => disturbed = true,
            _ => {}
        }
        if sync_queries::completed(response.parsed(), &tag)? {
            return Ok(Changes {
                fetched,
                vanished: Ranges::merged(ranges),
                disturbed,
            });
        }
    }
}

fn bounds(range: &RangeInclusive<u32>) -> (u32, u32) {
    let (start, end) = (*range.start(), *range.end());
    (start.min(end), start.max(end))
}

/// Publishes the cached identities a QRESYNC check found expunged.
#[cfg(feature = "condstore")]
pub(super) async fn publish(
    output: &Sender<MailSyncItem>,
    account: &str,
    folder: &str,
    ids: Vec<String>,
) -> anyhow::Result<()> {
    output
        .send(MailSyncItem::Vanished {
            account: account.to_owned(),
            folder: folder.to_owned(),
            ids,
        })
        .await
        .context("Sync was cancelled")
}

/// Without persistence a check never has a saved state, so never resyncs.
#[cfg(not(feature = "condstore"))]
pub(super) async fn publish(
    _output: &Sender<MailSyncItem>,
    _account: &str,
    _folder: &str,
    _ids: Vec<String>,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge_overlaps_and_neighbours() {
        let ranges = Ranges::merged(vec![(9, 12), (1, 3), (4, 4), (11, 15), (20, 20)]);
        assert_eq!(ranges, Ranges(vec![(1, 4), (9, 15), (20, 20)]));
        for uid in [1, 4, 9, 15, 20] {
            assert!(ranges.contains(uid), "{uid}");
        }
        for uid in [0, 5, 8, 16, 19, 21, u32::MAX] {
            assert!(!ranges.contains(uid), "{uid}");
        }
        assert!(Ranges::merged(vec![(u32::MAX, u32::MAX), (1, u32::MAX)]).contains(u32::MAX));
    }

    #[test]
    fn index_groups_cached_uids_by_folder_and_validity() {
        let known: HashSet<String> = [
            "acc:INBOX:12.5",
            "acc:INBOX:12.6",
            "acc:INBOX:11.5",
            "acc:Work:Clients:3.9",
            "acc:INBOX:local-sent-1",
            "other:INBOX:12.7",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let index = Index::new("acc", &known);
        assert_eq!(index.cached("INBOX", 12), HashSet::from([5, 6]));
        assert_eq!(index.cached("INBOX", 11), HashSet::from([5]));
        assert_eq!(index.cached("Work:Clients", 3), HashSet::from([9]));
        assert!(index.cached("Archive", 12).is_empty());
    }

    #[test]
    fn reconcile_removes_vanished_and_finds_new_mail_when_the_size_matches() {
        let cached = HashSet::from([4, 5, 6, 7]);
        let vanished = Ranges::merged(vec![(1, 5)]);
        assert_eq!(
            reconcile(&cached, &vanished, [6, 8, 9, 3], 4),
            Some(Outcome {
                vanished: vec![4, 5],
                new: vec![9, 8],
            }),
            "cached 4 - vanished 2 + new 2 = EXISTS 4; a vanished report is never new"
        );
    }

    #[test]
    fn a_size_that_does_not_add_up_needs_the_complete_listing() {
        let cached = HashSet::from([6, 7]);
        assert_eq!(reconcile(&cached, &Ranges::default(), [], 3), None);
        assert_eq!(reconcile(&cached, &Ranges::default(), [8], 2), None);
        assert!(reconcile(&cached, &Ranges::default(), [], 2).is_some());
    }
}
