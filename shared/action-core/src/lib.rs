//! Shared ownership of an action's optimistic display until its result is observed.

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
    use super::Projection;

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
