//! Test-support comparison of the displayed mail page against the store. The
//! controller asks the store for its own answer to the current query whenever
//! a backend event or dispatched command could have changed it, keeping at most
//! one request in flight, and reports whether the projected page agrees.
use super::*;
use crate::store::truth::StoreTruth;
use std::collections::BTreeMap;

pub(super) struct State {
    enabled: bool,
    latest: Option<Arc<StoreTruth>>,
    /// Backend events and dispatched commands since launch.
    changes: u64,
    /// The change count the latest answer was requested at.
    seen: u64,
    inflight: Option<u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            enabled: !std::env::args().any(|arg| arg == "--no-store-truth"),
            latest: None,
            changes: 0,
            seen: 0,
            inflight: None,
        }
    }
}

impl State {
    pub fn changed(&mut self) {
        self.changes += 1;
    }
    fn fresh(&self) -> bool {
        self.latest.is_some() && self.inflight.is_none() && self.seen == self.changes
    }
}

fn nonzero(counts: &BTreeMap<String, usize>) -> BTreeMap<&str, usize> {
    counts
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(account, count)| (account.as_str(), *count))
        .collect()
}

impl App {
    /// Ask for the store's answer when something may have changed it. Only an
    /// observed fixture run sends this; unit tests see the ordinary commands.
    pub(super) fn request_store_truth(&mut self) {
        if self.test_state.is_none() || !self.store_truth.enabled {
            return;
        }
        let state = &self.store_truth;
        if state.inflight.is_some() || (state.latest.is_some() && state.seen == state.changes) {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        let revision = state.changes;
        if tx
            .try_send(Command::StoreTruth(revision, self.query.clone()))
            .is_ok()
        {
            self.store_truth.inflight = Some(revision);
        }
    }

    pub(super) fn store_truth_received(&mut self, revision: u64, truth: Arc<StoreTruth>) {
        if self.store_truth.inflight != Some(revision) {
            return;
        }
        self.store_truth.inflight = None;
        self.store_truth.seen = revision;
        self.store_truth.latest = Some(truth);
    }

    pub(super) fn store_truth_observation(&self) -> serde_json::Value {
        let state = &self.store_truth;
        if !state.enabled {
            return serde_json::json!({"enabled": false, "fresh": false, "agrees": false});
        }
        let Some(truth) = &state.latest else {
            return serde_json::json!({"fresh": false, "agrees": false, "changes": state.changes});
        };
        let page_matches = truth.page_ids.iter().map(String::as_str).eq(self
            .page
            .rows
            .iter()
            .map(|mail| mail.id.as_str()));
        let counts_match = truth.total == self.page.total
            && truth.unread == self.page.unread
            && nonzero(&truth.inbox_unread) == nonzero(&self.page.inbox_unread);
        let fresh = state.fresh();
        serde_json::json!({
            "fresh": fresh,
            "page_matches": page_matches,
            "counts_match": counts_match,
            "agrees": fresh && page_matches && counts_match,
            "badge": mail_actions::badge_total(
                &self.workspace.accounts,
                &truth.inbox_unread,
                self.preferences.unread_badge,
            ),
            "revision": state.seen,
            "changes": state.changes,
            "truth": &**truth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::truth::FolderCount;

    #[test]
    fn pixel_runs_can_disable_the_expensive_oracle_without_claiming_agreement() {
        let (sender, mut commands) = engine::CommandSender::foreground_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.test_state = Some(std::path::PathBuf::from("unused"));
        app.store_truth.enabled = false;
        app.store_truth.changed();
        app.request_store_truth();
        assert!(commands.try_recv().is_err());
        assert_eq!(app.store_truth_observation()["enabled"], false);
        assert_eq!(app.store_truth_observation()["agrees"], false);
        app.store_truth.enabled = true;
        app.request_store_truth();
        assert!(matches!(commands.try_recv(), Ok(Command::StoreTruth(..))));
    }

    fn truth(
        ids: &[&str],
        total: usize,
        unread: usize,
        inbox: &[(&str, usize)],
    ) -> Arc<StoreTruth> {
        Arc::new(StoreTruth {
            page_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            total,
            unread,
            inbox_unread: inbox
                .iter()
                .map(|(account, count)| ((*account).to_owned(), *count))
                .collect(),
            folders: vec![FolderCount::default()],
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn agreement_needs_a_fresh_answer_matching_rows_and_counts() {
        let store = crate::store::Store::memory().unwrap();
        let mail = parse_mail(
            "fixture",
            "1",
            "INBOX",
            b"From: fixture@example.test\r\nSubject: Truth\r\n\r\nbody".to_vec(),
            true,
            false,
        )
        .unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let (sender, mut commands) = engine::CommandSender::foreground_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.folder = "INBOX".into();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        app.request_store_truth();
        assert!(
            commands.try_recv().is_err(),
            "no truth request without an observed run"
        );
        app.test_state = Some(std::path::PathBuf::from("unused"));
        app.store_truth.changed();
        app.request_store_truth();
        let Command::StoreTruth(revision, query) = commands.try_recv().unwrap() else {
            panic!("expected a truth request");
        };
        assert_eq!(query.folder, "INBOX");
        app.request_store_truth();
        assert!(
            commands.try_recv().is_err(),
            "one request in flight at a time"
        );
        app.store_truth_received(revision + 1, truth(&[], 0, 0, &[]));
        assert!(!app.store_truth_observation()["fresh"].as_bool().unwrap());
        app.store_truth_received(revision, truth(&[&id], 1, 1, &[("fixture", 1)]));
        let observation = app.store_truth_observation();
        assert_eq!(observation["fresh"], true);
        assert_eq!(observation["agrees"], true);
        app.store_truth.changed();
        let observation = app.store_truth_observation();
        assert_eq!(observation["fresh"], false);
        assert_eq!(observation["page_matches"], true);
        assert_eq!(observation["agrees"], false);
        app.request_store_truth();
        let Command::StoreTruth(revision, _) = commands.try_recv().unwrap() else {
            panic!("expected a second truth request");
        };
        app.store_truth_received(revision, truth(&[&id], 1, 0, &[]));
        let observation = app.store_truth_observation();
        assert_eq!(observation["fresh"], true);
        assert_eq!(observation["counts_match"], false);
        assert_eq!(observation["agrees"], false);
    }
}
