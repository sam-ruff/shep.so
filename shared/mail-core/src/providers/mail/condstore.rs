//! CONDSTORE (RFC 7162) flag refresh. A folder whose saved HIGHESTMODSEQ and
//! UIDVALIDITY still match the server fetches only flags changed since that
//! point; anything unexpected falls back to fetching every message's flags.
use super::*;
use std::collections::HashMap;

/// The server state a check fully observed for one folder: every cached
/// message's flags reflect the server at `modseq` or changed after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderState {
    pub validity: u32,
    pub modseq: u64,
}

/// Saved folder states for one account, keyed by exact wire folder name.
pub type Resume = HashMap<String, FolderState>;

/// How a check refreshes the flags of already cached messages in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Plan {
    /// Fetch flags for every listed message.
    Full,
    /// Nothing changed since the saved state.
    Unchanged,
    /// Fetch only flags whose mod-sequence exceeds `since`.
    Changes { since: u64 },
}

/// RFC 7162 mod-sequences are positive and below 2^63; anything else cannot
/// be stored or compared safely.
pub(super) fn usable(highest: Option<u64>) -> Option<u64> {
    highest.filter(|value| *value > 0 && *value <= i64::MAX as u64)
}

/// Chooses the refresh for a folder whose SELECT reported `validity` and
/// `highest`. A missing saved state, a new UIDVALIDITY, NOMODSEQ or a
/// HIGHESTMODSEQ below the saved one all need the full path.
pub(super) fn plan(saved: Option<&FolderState>, validity: u32, highest: Option<u64>) -> Plan {
    let (Some(saved), Some(highest)) = (saved, usable(highest)) else {
        return Plan::Full;
    };
    if saved.validity != validity || highest < saved.modseq {
        return Plan::Full;
    }
    if highest == saved.modseq {
        return Plan::Unchanged;
    }
    Plan::Changes {
        since: saved.modseq,
    }
}

/// Flags of messages changed after `since`, checked against the tagged
/// completion so partial data before NO/BAD is never used.
pub(super) async fn changed_flags<T>(
    session: &mut async_imap::Session<T>,
    since: u64,
) -> anyhow::Result<Vec<sync_queries::Fetch>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    sync_queries::fetch(
        session,
        "1:*",
        &format!("(UID FLAGS) (CHANGEDSINCE {since})"),
    )
    .await
}

/// Publishes a folder's new saved state, or `None` to forget it.
#[cfg(feature = "condstore")]
pub(super) async fn publish(
    output: &Sender<MailSyncItem>,
    account: &str,
    folder: &str,
    state: Option<FolderState>,
) -> anyhow::Result<()> {
    output
        .send(MailSyncItem::FolderState {
            account: account.to_owned(),
            folder: folder.to_owned(),
            state,
        })
        .await
        .context("Sync was cancelled")
}

/// Without persistence there is never a saved state to publish.
#[cfg(not(feature = "condstore"))]
pub(super) async fn publish(
    _output: &Sender<MailSyncItem>,
    _account: &str,
    _folder: &str,
    _state: Option<FolderState>,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVED: FolderState = FolderState {
        validity: 12,
        modseq: 100,
    };

    #[test]
    fn a_matching_saved_state_fetches_only_later_changes() {
        assert_eq!(
            plan(Some(&SAVED), 12, Some(140)),
            Plan::Changes { since: 100 }
        );
        assert_eq!(plan(Some(&SAVED), 12, Some(100)), Plan::Unchanged);
    }

    #[test]
    fn anything_unexpected_uses_the_full_path() {
        assert_eq!(plan(None, 12, Some(140)), Plan::Full, "no saved state");
        assert_eq!(plan(Some(&SAVED), 13, Some(140)), Plan::Full, "UIDVALIDITY");
        assert_eq!(plan(Some(&SAVED), 12, None), Plan::Full, "NOMODSEQ");
        assert_eq!(
            plan(Some(&SAVED), 12, Some(99)),
            Plan::Full,
            "went backwards"
        );
        assert_eq!(plan(Some(&SAVED), 12, Some(0)), Plan::Full, "zero");
        assert_eq!(
            plan(Some(&SAVED), 12, Some(u64::MAX)),
            Plan::Full,
            "outside the RFC 7162 range"
        );
    }
}
