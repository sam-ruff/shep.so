//! Shared ownership of an action's optimistic display until its result is observed.

/// Persisted execution status. Domain owners retain receipts and recovery proofs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Rejected,
    Uncertain,
    Repair,
    Cancelled,
}

impl Status {
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Running,
        Self::Waiting,
        Self::Succeeded,
        Self::Rejected,
        Self::Uncertain,
        Self::Repair,
        Self::Cancelled,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Succeeded => "succeeded",
            Self::Rejected => "rejected",
            Self::Uncertain => "uncertain",
            Self::Repair => "repair",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.key() == value)
    }

    /// Call only after excluding the previous execution owner.
    pub fn after_restart(self) -> Self {
        if self == Self::Running {
            Self::Uncertain
        } else {
            self
        }
    }

    pub fn can_dispatch(self) -> bool {
        self == Self::Queued
    }

    pub fn needs_review(self) -> bool {
        matches!(self, Self::Rejected | Self::Uncertain | Self::Repair)
    }

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Projection {
    #[default]
    Pending,
    Committed {
        revision: u64,
    },
    /// The provider accepted the write, but local application needs repair.
    Repair,
    Rejected,
    Uncertain,
}

impl Projection {
    pub fn acknowledge(revision: Option<u64>) -> Self {
        revision.map_or(Self::Repair, |revision| Self::Committed { revision })
    }

    pub fn is_visible(self) -> bool {
        !matches!(self, Self::Rejected)
    }

    pub fn observed_at(self, revision: u64) -> bool {
        matches!(self, Self::Committed { revision: committed } if revision >= committed)
    }

    pub fn can_retry(self) -> bool {
        matches!(self, Self::Rejected)
    }

    pub fn needs_review(self) -> bool {
        matches!(self, Self::Repair | Self::Rejected | Self::Uncertain)
    }
}

#[cfg(test)]
mod tests {
    use super::{Projection, Status};

    #[test]
    fn restart_never_authorises_another_unconfirmed_provider_write() {
        for status in Status::ALL {
            let recovered = status.after_restart();
            assert_eq!(recovered.can_dispatch(), status == Status::Queued);
            if status == Status::Running {
                assert_eq!(recovered, Status::Uncertain);
                assert!(recovered.needs_review());
            } else {
                assert_eq!(recovered, status);
            }
        }
        assert_eq!(Status::parse("future-status"), None);
    }

    #[test]
    fn durable_status_keys_round_trip_without_losing_recovery_states() {
        for status in Status::ALL {
            assert_eq!(Status::parse(status.key()), Some(status));
        }
        for status in [Status::Repair, Status::Uncertain, Status::Rejected] {
            assert!(status.needs_review());
            assert!(!status.is_finished());
        }
        assert!(Status::Succeeded.is_finished());
        assert!(Status::Cancelled.is_finished());
    }

    #[test]
    fn old_snapshots_keep_a_committed_effect_until_its_revision_is_visible() {
        let pending = Projection::Pending;
        assert!(pending.is_visible());
        assert!(!pending.observed_at(u64::MAX));
        let acknowledged = Projection::acknowledge(Some(9));
        assert!(acknowledged.is_visible());
        assert!(!acknowledged.observed_at(8));
        assert!(acknowledged.observed_at(9));
        assert!(acknowledged.observed_at(10));
    }

    #[test]
    fn cache_repair_and_unknown_outcomes_cannot_be_retried_as_rejections() {
        for state in [Projection::acknowledge(None), Projection::Uncertain] {
            assert!(state.is_visible());
            assert!(state.needs_review());
            assert!(!state.can_retry());
            assert!(!state.observed_at(u64::MAX));
        }
        assert!(!Projection::Rejected.is_visible());
        assert!(Projection::Rejected.can_retry());
        assert!(Projection::Rejected.needs_review());
    }
}
