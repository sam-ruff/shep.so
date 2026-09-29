//! Background mail-check failure episodes. Each account's episode is bound to
//! its incoming connection identity (server settings plus credential slot),
//! so unrelated workspace changes such as a folder listing or a rename never
//! make a result stale, while a reconfigured or removed account retires it.
use crate::engine::{SyncAttempt, SyncOrigin};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const GRACE: Duration = Duration::from_secs(30);
/// The notice owner for a delayed banner covering several accounts.
pub(super) const COMBINED: &str = "*";

/// The account listing and the demo target have no saved connection identity.
fn unbound(account: &str) -> bool {
    account == "accounts" || account == "preview"
}

#[derive(Default)]
pub(super) struct State {
    latest: HashMap<String, (u64, String)>,
    failures: HashMap<String, Failure>,
}

struct Failure {
    first: Instant,
    connection: String,
    error: String,
    dismissed: bool,
    displayed: bool,
}

impl State {
    /// Records the newest attempt. Returns true when it uses another
    /// connection identity than the previous one, retiring that episode.
    pub(super) fn started(&mut self, attempt: &SyncAttempt) -> bool {
        let replaced = self
            .latest
            .get(&attempt.account)
            .is_some_and(|(_, connection)| connection != &attempt.connection);
        if replaced {
            self.failures.remove(&attempt.account);
        }
        self.latest.insert(
            attempt.account.clone(),
            (attempt.sequence, attempt.connection.clone()),
        );
        replaced
    }

    /// A result counts only for the account's current identity and newest
    /// attempt. `identity` is the account's identity in the current
    /// workspace, or `None` once the account is removed.
    pub(super) fn current(&self, attempt: &SyncAttempt, identity: Option<&str>) -> bool {
        (unbound(&attempt.account) || identity == Some(attempt.connection.as_str()))
            && self
                .latest
                .get(&attempt.account)
                .is_none_or(|(sequence, connection)| {
                    *sequence <= attempt.sequence && connection == &attempt.connection
                })
    }

    /// Success ends the episode. A background failure starts one, or keeps
    /// the first failure time of an existing one; a Refresh failure is shown
    /// immediately by the caller and neither starts nor extends the timer.
    pub(super) fn finished(&mut self, attempt: &SyncAttempt, error: Option<&str>, now: Instant) {
        let Some(error) = error else {
            self.failures.remove(&attempt.account);
            return;
        };
        if let Some(failure) = self.failures.get_mut(&attempt.account) {
            failure.error = error.to_owned();
        } else if attempt.origin == SyncOrigin::Background {
            self.failures.insert(
                attempt.account.clone(),
                Failure {
                    first: now,
                    connection: attempt.connection.clone(),
                    error: error.to_owned(),
                    dismissed: false,
                    displayed: false,
                },
            );
        }
    }

    /// Marks episodes that have failed continuously for the grace period.
    /// Returns true when at least one should now be shown.
    pub(super) fn due(&mut self, now: Instant) -> bool {
        let mut due = false;
        for failure in self.failures.values_mut() {
            if !failure.dismissed
                && !failure.displayed
                && now.saturating_duration_since(failure.first) >= GRACE
            {
                failure.displayed = true;
                due = true;
            }
        }
        due
    }

    /// The only failing account and its latest error, when exactly one is.
    pub(super) fn single_failure(&self) -> Option<(&str, &str)> {
        let mut failures = self.failures.iter();
        let (account, failure) = failures.next()?;
        failures
            .next()
            .is_none()
            .then_some((account.as_str(), failure.error.as_str()))
    }

    pub(super) fn dismiss(&mut self, account: &str) {
        if account == COMBINED {
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

    /// Moves an episode's first failure back, standing in for elapsed time.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, account: &str, by: Duration) {
        if let Some(failure) = self.failures.get_mut(account) {
            failure.first -= by;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn failure_count(&self) -> usize {
        self.failures.len()
    }

    /// Applies a new workspace. Episodes whose account was removed or whose
    /// incoming identity changed are retired and returned; renames and other
    /// account edits keep them. Removed accounts also forget their newest
    /// attempt, so none of their late results count.
    pub(super) fn reconcile<'a>(
        &mut self,
        identity: impl Fn(&str) -> Option<&'a str>,
    ) -> Vec<String> {
        let mut retired = Vec::new();
        self.failures.retain(|account, failure| {
            let keep = unbound(account) || identity(account) == Some(failure.connection.as_str());
            if !keep {
                retired.push(account.clone());
            }
            keep
        });
        self.latest
            .retain(|account, _| unbound(account) || identity(account).is_some());
        retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(account: &str, sequence: u64, origin: SyncOrigin) -> SyncAttempt {
        SyncAttempt {
            account: account.into(),
            connection: "slot".into(),
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
        state.finished(&a, Some("down"), first);
        state.finished(&a, Some("still down"), first + Duration::from_secs(20));
        assert!(!state.due(first + Duration::from_secs(29)));
        assert!(state.due(first + GRACE));
        assert_eq!(state.single_failure(), Some(("a", "still down")));
        state.dismiss("a");
        state.finished(&a, Some("down"), first + Duration::from_secs(60));
        assert!(!state.due(first + Duration::from_secs(61)));
        state.finished(&a, None, first + Duration::from_secs(62));
        state.finished(&a, Some("down"), first + Duration::from_secs(63));
        assert!(!state.due(first + Duration::from_secs(92)));
        assert!(state.due(first + Duration::from_secs(93)));
    }

    #[test]
    fn accounts_manual_and_stale_attempts_are_independent() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        let b = attempt("b", 2, SyncOrigin::Background);
        state.started(&a);
        state.started(&b);
        state.finished(&a, Some("down"), first);
        state.finished(&b, Some("down"), first + Duration::from_secs(20));
        assert_eq!(state.single_failure(), None, "two accounts are failing");
        state.finished(
            &attempt("a", 3, SyncOrigin::Refresh),
            Some("down"),
            first + Duration::from_secs(25),
        );
        assert!(state.due(first + GRACE));
        state.finished(&b, None, first + Duration::from_secs(31));
        assert!(!state.due(first + Duration::from_secs(50)));
        assert!(state.current(&a, Some("slot")));
        state.started(&attempt("a", 4, SyncOrigin::Background));
        assert!(
            !state.current(&a, Some("slot")),
            "an older attempt is stale"
        );
        assert!(!state.current(&attempt("a", 4, SyncOrigin::Background), Some("new-slot")));
        assert!(!state.current(&attempt("a", 4, SyncOrigin::Background), None));
    }

    #[test]
    fn a_refresh_failure_neither_starts_nor_extends_the_timer() {
        let first = Instant::now();
        let mut state = State::default();
        let manual = attempt("a", 1, SyncOrigin::Refresh);
        state.started(&manual);
        state.finished(&manual, Some("down"), first);
        assert!(!state.has_failures());
        let background = attempt("a", 2, SyncOrigin::Background);
        state.finished(&background, Some("down"), first + Duration::from_secs(5));
        state.finished(&manual, Some("down"), first + Duration::from_secs(20));
        assert!(state.due(first + Duration::from_secs(35)), "timer kept");
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
        state.finished(&a, Some("down"), first);
        state.finished(&b, Some("down"), first);
        let mut replacement = attempt("a", 3, SyncOrigin::Background);
        replacement.connection = "new-slot".into();
        assert!(state.started(&replacement));
        assert!(!state.current(&a, Some("new-slot")));
        assert!(state.due(first + GRACE), "b keeps its original timer");
        let retired = state.reconcile(|account| (account == "a").then_some("new-slot"));
        assert_eq!(retired, vec!["b".to_owned()]);
        assert!(
            !state.current(&b, None),
            "a removed account's result is stale"
        );
        state.finished(&replacement, Some("down"), first + Duration::from_secs(31));
        assert!(!state.due(first + Duration::from_secs(60)));
        assert!(state.due(first + Duration::from_secs(61)));
    }

    #[test]
    fn reconcile_keeps_episodes_whose_identity_is_unchanged() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        state.started(&a);
        state.finished(&a, Some("down"), first);
        assert!(state.reconcile(|_| Some("slot")).is_empty());
        assert!(state.current(&a, Some("slot")));
        assert!(state.due(first + GRACE));
        assert_eq!(
            state.reconcile(|_| Some("reconnected")),
            vec!["a".to_owned()]
        );
        assert!(!state.has_failures());
    }

    #[test]
    fn dismissing_an_immediate_refresh_error_suppresses_the_automatic_episode() {
        let first = Instant::now();
        let mut state = State::default();
        let background = attempt("a", 1, SyncOrigin::Background);
        state.started(&background);
        state.finished(&background, Some("down"), first);
        let manual = attempt("a", 2, SyncOrigin::Refresh);
        state.started(&manual);
        state.finished(&manual, Some("down"), first + Duration::from_secs(10));
        state.dismiss("a");
        assert!(!state.due(first + Duration::from_secs(40)));
        state.finished(&manual, None, first + Duration::from_secs(41));
        let next = attempt("a", 3, SyncOrigin::Background);
        state.started(&next);
        state.finished(&next, Some("down"), first + Duration::from_secs(42));
        assert!(state.due(first + Duration::from_secs(72)));
    }

    #[test]
    fn dismissing_one_notice_preserves_another_accounts_unseen_failure() {
        let first = Instant::now();
        let mut state = State::default();
        let a = attempt("a", 1, SyncOrigin::Background);
        let b = attempt("b", 2, SyncOrigin::Background);
        state.started(&a);
        state.started(&b);
        state.finished(&a, Some("down"), first);
        state.finished(&b, Some("down"), first + Duration::from_secs(20));
        assert!(state.due(first + GRACE));
        state.dismiss(COMBINED);
        assert!(state.due(first + Duration::from_secs(50)));
        state.dismiss(COMBINED);
        assert!(!state.due(first + Duration::from_secs(51)));
    }
}
