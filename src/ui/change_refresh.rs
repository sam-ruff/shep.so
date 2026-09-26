//! Coalesces cache change notices so a busy database worker is not flooded
//! with list, conversation and reader reads it cannot answer in time.
use std::time::{Duration, Instant};

/// A refresh whose reads never answer (a failed read) stops holding later ones.
const STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct State {
    outstanding: Option<Outstanding>,
    again: bool,
}

struct Outstanding {
    page: Option<u64>,
    conversation: Option<u64>,
    detail: Option<u64>,
    since: Instant,
}

impl State {
    /// True when a change must wait for the refresh already in flight.
    pub(super) fn defer(&mut self, now: Instant) -> bool {
        if self.blocked(now) {
            self.again = true;
        }
        self.again
    }

    pub(super) fn started(
        &mut self,
        page: Option<u64>,
        conversation: Option<u64>,
        detail: Option<u64>,
        now: Instant,
    ) {
        self.again = false;
        self.outstanding = Some(Outstanding {
            page,
            conversation,
            detail,
            since: now,
        });
        self.settle(|_| {});
    }

    pub(super) fn page_answered(&mut self, generation: u64) {
        self.settle(|o| clear(&mut o.page, generation));
    }

    pub(super) fn conversation_answered(&mut self, generation: u64) {
        self.settle(|o| clear(&mut o.conversation, generation));
    }

    pub(super) fn detail_answered(&mut self, revision: u64) {
        self.settle(|o| clear(&mut o.detail, revision));
    }

    /// A deferred change is due once the previous refresh has settled.
    pub(super) fn due(&mut self, now: Instant) -> bool {
        self.again && !self.blocked(now)
    }

    fn blocked(&self, now: Instant) -> bool {
        self.outstanding
            .as_ref()
            .is_some_and(|o| now.saturating_duration_since(o.since) < STALE_AFTER)
    }

    fn settle(&mut self, answer: impl FnOnce(&mut Outstanding)) {
        let Some(outstanding) = &mut self.outstanding else {
            return;
        };
        answer(outstanding);
        if outstanding.page.is_none()
            && outstanding.conversation.is_none()
            && outstanding.detail.is_none()
        {
            self.outstanding = None;
        }
    }
}

/// A later answer also covers an earlier request that was superseded.
fn clear(slot: &mut Option<u64>, answered: u64) {
    if slot.is_some_and(|requested| answered >= requested) {
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_during_a_refresh_collapse_into_one_follow_up() {
        let now = Instant::now();
        let mut state = State::default();
        assert!(!state.defer(now));
        state.started(Some(4), Some(2), Some(7), now);
        for _ in 0..20 {
            assert!(state.defer(now));
        }
        assert!(!state.due(now));
        state.page_answered(4);
        state.conversation_answered(2);
        assert!(!state.due(now), "the reader is still loading");
        state.detail_answered(7);
        assert!(state.due(now));
        state.started(Some(5), None, None, now);
        assert!(!state.due(now));
        state.page_answered(5);
        assert!(!state.due(now), "nothing changed since the last refresh");
        assert!(!state.defer(now));
    }

    #[test]
    fn older_answers_do_not_settle_and_newer_ones_do() {
        let now = Instant::now();
        let mut state = State::default();
        state.started(Some(4), None, None, now);
        state.page_answered(3);
        assert!(state.defer(now));
        state.page_answered(9);
        assert!(state.due(now));
    }

    #[test]
    fn a_refresh_that_never_answers_stops_blocking() {
        let now = Instant::now();
        let mut state = State::default();
        state.started(Some(1), None, None, now);
        assert!(state.defer(now));
        assert!(!state.due(now + STALE_AFTER - Duration::from_millis(1)));
        assert!(state.due(now + STALE_AFTER));
    }

    #[test]
    fn a_refresh_that_sent_nothing_is_settled_at_once() {
        let now = Instant::now();
        let mut state = State::default();
        state.started(None, None, None, now);
        assert!(!state.defer(now));
    }
}
