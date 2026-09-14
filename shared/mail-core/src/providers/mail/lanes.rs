//! Body download lanes: small new messages first, large ones afterwards.
use crate::model::MAX_MESSAGE_BYTES;
use std::ops::Range;

/// Bodies above this size wait behind every folder's small messages. One
/// fetch of this size takes longer on an ordinary link than a whole batch
/// of small bodies, so it must not sit in front of them.
pub const SLOW_LANE_BYTES: usize = 1024 * 1024;
pub(super) const BATCH_MESSAGES: usize = 10;
pub(super) const BATCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    /// Fetched in batches right after its metadata chunk.
    Inline,
    /// Fetched one at a time after every folder's inline bodies.
    Slow,
    /// Streamed to private staging after the slow lane.
    Staged,
}

pub(super) fn lane(size: Option<u32>) -> Lane {
    let size = size.map_or(usize::MAX, |size| size as usize);
    if size > MAX_MESSAGE_BYTES {
        return Lane::Staged;
    }
    if size > SLOW_LANE_BYTES {
        return Lane::Slow;
    }
    Lane::Inline
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pending {
    pub uid: u32,
    pub size: usize,
}

/// Smallest first so a run of small new messages never waits behind a large
/// one; equal sizes keep the newest (highest UID) first.
pub(super) fn order_inline(pending: &mut [Pending]) {
    pending.sort_unstable_by(|a, b| a.size.cmp(&b.size).then(b.uid.cmp(&a.uid)));
}

/// Greedy batches of at most `BATCH_MESSAGES` bodies or `BATCH_BYTES`; a
/// single body larger than the byte budget still forms its own batch.
pub(super) fn batches(pending: &[Pending]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset < pending.len() {
        let start = offset;
        let mut bytes = 0;
        while offset < pending.len() && offset - start < BATCH_MESSAGES {
            let next = pending[offset].size;
            if offset > start && bytes + next > BATCH_BYTES {
                break;
            }
            bytes += next;
            offset += 1;
        }
        ranges.push(start..offset);
    }
    ranges
}

/// A body left for a later lane, with the flags seen when it was listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Deferred {
    pub folder_index: usize,
    pub folder: String,
    pub validity: u32,
    pub uid: u32,
    pub size: u32,
    pub unread: bool,
    pub starred: bool,
}

impl Deferred {
    pub fn inbox(&self) -> bool {
        self.folder.eq_ignore_ascii_case("INBOX")
    }
}

/// Folder order first (Inbox leads), then smallest first, newest first.
pub(super) fn order_slow(deferred: &mut [Deferred]) {
    deferred.sort_by(|a, b| {
        a.folder_index
            .cmp(&b.folder_index)
            .then(a.size.cmp(&b.size))
            .then(b.uid.cmp(&a.uid))
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InboxFinish {
    Inline,
    SlowLane,
    Staged,
}

/// `InboxSyncFinished` follows the last Inbox body, whichever lane carries it,
/// so a first import never reports its own backlog as new arrivals.
pub(super) fn inbox_finish(slow: &[Deferred], staged: &[Deferred]) -> InboxFinish {
    if staged.iter().any(Deferred::inbox) {
        return InboxFinish::Staged;
    }
    if slow.iter().any(Deferred::inbox) {
        return InboxFinish::SlowLane;
    }
    InboxFinish::Inline
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(items: &[(u32, usize)]) -> Vec<Pending> {
        items
            .iter()
            .map(|&(uid, size)| Pending { uid, size })
            .collect()
    }

    fn deferred(folder: &str, folder_index: usize, uid: u32, size: u32) -> Deferred {
        Deferred {
            folder_index,
            folder: folder.into(),
            validity: 1,
            uid,
            size,
            unread: true,
            starred: false,
        }
    }

    #[test]
    fn lane_splits_on_the_slow_and_staged_limits() {
        assert_eq!(lane(Some(0)), Lane::Inline);
        assert_eq!(lane(Some(SLOW_LANE_BYTES as u32)), Lane::Inline);
        assert_eq!(lane(Some(SLOW_LANE_BYTES as u32 + 1)), Lane::Slow);
        assert_eq!(lane(Some(MAX_MESSAGE_BYTES as u32)), Lane::Slow);
        assert_eq!(lane(Some(MAX_MESSAGE_BYTES as u32 + 1)), Lane::Staged);
        assert_eq!(lane(None), Lane::Staged);
    }

    #[test]
    fn inline_order_is_smallest_first_then_newest() {
        let mut items = pending(&[(9, 300), (8, 100), (7, 300), (6, 100), (5, 200)]);
        order_inline(&mut items);
        assert_eq!(
            items,
            pending(&[(8, 100), (6, 100), (5, 200), (9, 300), (7, 300)])
        );
    }

    #[test]
    fn batches_cap_message_count_and_bytes_without_stranding_a_large_body() {
        let mut items = pending(&[(1, 1); 12]);
        assert_eq!(batches(&items), vec![0..10, 10..12]);
        items = pending(&[(1, 3 * 1024 * 1024), (2, 2 * 1024 * 1024), (3, 1)]);
        assert_eq!(batches(&items), vec![0..1, 1..3]);
        items = pending(&[(1, 5 * 1024 * 1024), (2, 1)]);
        assert_eq!(batches(&items), vec![0..1, 1..2]);
        assert!(batches(&[]).is_empty());
    }

    #[test]
    fn slow_order_keeps_inbox_first_then_smallest_then_newest() {
        let mut items = vec![
            deferred("Archive", 1, 3, 10),
            deferred("INBOX", 0, 4, 50),
            deferred("INBOX", 0, 5, 20),
            deferred("INBOX", 0, 6, 20),
        ];
        order_slow(&mut items);
        let order: Vec<_> = items.iter().map(|item| item.uid).collect();
        assert_eq!(order, [6, 5, 4, 3]);
    }

    #[test]
    fn inbox_finishes_after_the_last_lane_carrying_inbox_mail() {
        let inbox = std::slice::from_ref(&deferred("INBOX", 0, 1, 1)).to_vec();
        let other = std::slice::from_ref(&deferred("Archive", 1, 2, 1)).to_vec();
        assert_eq!(inbox_finish(&[], &[]), InboxFinish::Inline);
        assert_eq!(inbox_finish(&other, &other), InboxFinish::Inline);
        assert_eq!(inbox_finish(&inbox, &other), InboxFinish::SlowLane);
        assert_eq!(inbox_finish(&inbox, &inbox), InboxFinish::Staged);
    }
}
