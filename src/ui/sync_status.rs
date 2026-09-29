use crate::engine::{SyncAttempt, SyncOrigin};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const GRACE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct State {
    latest: HashMap<String, (u64, String)>,
    failures: HashMap<String, Failure>,
}

struct Failure {
    first: Instant,
    revision: u64,
    dismissed: bool,
    displayed: bool,
}

impl State {
    pub(super) fn started(&mut self, attempt: &SyncAttempt) -> bool {
        let replaced = self
            .latest
            .get(&attempt.account)
            .is_some_and(|(_, connection)| connection != &attempt.connection);
        if replaced {
            self.failures.remove(&attempt.account);
        }
        if let Some(failure) = self.failures.get_mut(&attempt.account) {
            failure.revision = attempt.connection_revision;
        }
        self.latest.insert(
            attempt.account.clone(),
            (attempt.sequence, attempt.connection.clone()),
        );
        replaced
    }

    pub(super) fn current(&self, attempt: &SyncAttempt, revision: u64, present: bool) -> bool {
        (attempt.account == "accounts"
            || attempt.account == "preview"
            || (present && attempt.connection_revision == revision))
            && self
                .latest
                .get(&attempt.account)
                .is_none_or(|(sequence, connection)| {
                    *sequence <= attempt.sequence && connection == &attempt.connection
                })
    }

    pub(super) fn finished(&mut self, attempt: &SyncAttempt, failed: bool, now: Instant) {
        if !failed {
            self.failures.remove(&attempt.account);
        } else if attempt.origin == SyncOrigin::Background {
            self.failures
                .entry(attempt.account.clone())
                .or_insert(Failure {
                    first: now,
                    revision: attempt.connection_revision,
                    dismissed: false,
                    displayed: false,
                });
        }
    }

    pub(super) fn due(&mut self, now: Instant, revision: u64) -> bool {
        let mut due = false;
        for (account, failure) in &mut self.failures {
            if !failure.dismissed
                && !failure.displayed
                && (account == "accounts" || account == "preview" || failure.revision == revision)
                && now.saturating_duration_since(failure.first) >= GRACE
            {
                failure.displayed = true;
                due = true;
            }
        }
        due
    }

    pub(super) fn dismiss(&mut self, account: &str) {
        if account == "accounts" {
            for failure in self
                .failures
                .values_mut()
                .filter(|failure| failure.displayed)
            {
                failure.dismissed = true;
            }
        } else if let Some(failure) = self.failures.get_mut(account) {
            failure.dismissed = true;
        }
    }

    pub(super) fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }

    #[cfg(feature = "test-support")]
    pub(super) fn failure_count(&self) -> usize {
        self.failures.len()
    }

    pub(super) fn remove(&mut self, account: &str) {
        self.failures.remove(account);
        self.latest.remove(account);
    }

    pub(super) fn retain_accounts(&mut self, accounts: impl Fn(&str) -> bool) {
        self.failures.retain(|account, _| {
            account == "accounts" || account == "preview" || accounts(account)
        });
        self.latest.retain(|account, _| {
            account == "accounts" || account == "preview" || accounts(account)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(account: &str, sequence: u64, origin: SyncOrigin) -> SyncAttempt {
        SyncAttempt {
            account: account.into(),
            connection: String::new(),
            connection_revision: 2,
            sequence,
            origin,
        }
    }

    #[test]
    fn continuous_failures_keep_first_time_and_dismiss_until_recovery() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        state.started(&a);
        state.finished(&a, true, first);
        state.finished(&a, true, first + Duration::from_secs(20));
        assert!(!state.due(first + Duration::from_secs(29), 2));
        assert!(state.due(first + GRACE, 2));
        state.dismiss("a");
        state.finished(&a, true, first + Duration::from_secs(60));
        assert!(!state.due(first + Duration::from_secs(61), 2));
        state.finished(&a, false, first + Duration::from_secs(62));
        state.finished(&a, true, first + Duration::from_secs(63));
        assert!(!state.due(first + Duration::from_secs(92), 2));
        assert!(state.due(first + Duration::from_secs(93), 2));
    }

    #[test]
    fn accounts_manual_and_stale_attempts_are_independent() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        let b = attempt("b", 2, SyncOrigin::Background);
        state.started(&a);
        state.started(&b);
        state.finished(&a, true, first);
        state.finished(&b, true, first + Duration::from_secs(20));
        state.finished(
            &attempt("a", 3, SyncOrigin::Refresh),
            true,
            first + Duration::from_secs(25),
        );
        assert!(state.due(first + GRACE, 2));
        state.finished(&b, false, first + Duration::from_secs(31));
        assert!(!state.due(first + Duration::from_secs(50), 2));
        assert!(!state.current(&a, 3, true));
        state.started(&attempt("a", 4, SyncOrigin::Background));
        assert!(!state.current(&a, 2, true));
        assert!(!state.current(&a, 3, false));
    }

    #[test]
    fn replacement_and_removal_retire_only_the_owned_episode() {
        let first = Instant::now();
        let mut state = State::default();
        let mut a = attempt("a", 1, SyncOrigin::Background);
        a.connection = "old-slot".into();
        let b = attempt("b", 2, SyncOrigin::Background);
        state.started(&a);
        state.started(&b);
        state.finished(&a, true, first);
        state.finished(&b, true, first);
        let mut replacement = attempt("a", 3, SyncOrigin::Background);
        replacement.connection = "new-slot".into();
        state.started(&replacement);
        assert!(!state.current(&a, 2, true));
        assert!(state.due(first + GRACE, 2), "b keeps its original timer");
        state.remove("b");
        state.finished(&replacement, true, first + Duration::from_secs(31));
        assert!(!state.due(first + Duration::from_secs(60), 2));
        assert!(state.due(first + Duration::from_secs(61), 2));
    }

    #[test]
    fn dismissing_an_immediate_refresh_error_suppresses_the_automatic_episode() {
        let first = Instant::now();
        let mut state = State::default();
        let background = attempt("a", 1, SyncOrigin::Background);
        state.started(&background);
        state.finished(&background, true, first);
        let manual = attempt("a", 2, SyncOrigin::Refresh);
        state.started(&manual);
        state.finished(&manual, true, first + Duration::from_secs(10));
        state.dismiss("a");
        assert!(!state.due(first + Duration::from_secs(40), 2));
        state.finished(&manual, false, first + Duration::from_secs(41));
        let next = attempt("a", 3, SyncOrigin::Background);
        state.started(&next);
        state.finished(&next, true, first + Duration::from_secs(42));
        assert!(state.due(first + Duration::from_secs(72), 2));
    }

    #[test]
    fn dismissing_one_notice_preserves_another_accounts_unseen_failure() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        let b = attempt("b", 2, SyncOrigin::Background);
        state.started(&a);
        state.started(&b);
        state.finished(&a, true, first);
        state.finished(&b, true, first + Duration::from_secs(20));
        assert!(state.due(first + GRACE, 2));
        state.dismiss("accounts");
        assert!(state.due(first + Duration::from_secs(50), 2));
        state.dismiss("accounts");
        assert!(!state.due(first + Duration::from_secs(51), 2));
    }
}
