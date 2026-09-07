//! Native list selection is independent of the message being read. Membership
//! stays in SQLite; iced holds one page and at most 32 unacknowledged gestures.
use super::*;
use crate::{
    engine::selections::Request,
    store::{MailSelectionId, SelectionChange, SelectionSnapshot},
};

#[derive(Default)]
pub(super) struct State {
    pub mode: bool,
    #[cfg(feature = "test-support")]
    pub draw_epoch: u64,
    #[cfg(feature = "test-support")]
    pub drawn_epoch: Arc<std::sync::atomic::AtomicU64>,
    pub anchor: Option<(String, u64)>,
    token: Option<MailSelectionId>,
    scope: Option<MailQuery>,
    pub snapshot: Option<Arc<SelectionSnapshot>>,
    backend: Option<MailSelectionId>,
    serial: u64,
    inflight: Option<(u64, Request)>,
    queue: VecDeque<Gesture>,
    observed: Vec<String>,
    retry: Option<Instant>,
    pub visible: HashSet<String>,
    pub count: usize,
}
#[derive(Clone)]
struct Gesture {
    change: SelectionChange,
    range: Option<(u64, u64)>,
}
fn scope(query: &MailQuery) -> MailQuery {
    let mut query = query.clone();
    query.offset = 0;
    query.observe.clear();
    query.observe_bulk.clear();
    query
}
impl State {
    pub fn busy(&self) -> bool {
        self.mode
            && (self.snapshot.is_none()
                || !self.queue.is_empty()
                || self
                    .inflight
                    .as_ref()
                    .is_some_and(|(_, request)| Some(request.id()) == self.token))
    }
    pub fn ready(&self) -> bool {
        self.mode && !self.busy() && self.count > 0
    }
    pub fn ready_for_drag(&self) -> bool {
        self.mode
            && self.count > 0
            && self.snapshot.is_some()
            && self.queue.is_empty()
            && self.inflight.as_ref().is_none_or(|(_, request)| {
                matches!(request, Request::Observe(_) | Request::Release(_))
            })
    }
    fn reset(&mut self) {
        #[cfg(feature = "test-support")]
        {
            self.draw_epoch += 1;
        }
        self.mode = false;
        self.anchor = None;
        self.token = None;
        self.scope = None;
        self.snapshot = None;
        self.queue.clear();
        self.observed.clear();
        self.visible.clear();
        self.count = 0;
        // Keep the sole in-flight request until its reply; pump then releases
        // its snapshot before starting another scope. Rapid navigation cannot
        // leak an ever-growing collection of abandoned database selections.
    }
    fn start(&mut self, query: &MailQuery) {
        self.reset();
        self.mode = true;
        self.token = Some(MailSelectionId::default());
        self.scope = Some(scope(query));
    }
    fn project(&mut self, page: &MailPage, offset: usize) {
        self.visible = self
            .snapshot
            .as_ref()
            .map(|s| s.visible.clone())
            .unwrap_or_default();
        self.visible
            .retain(|id| page.rows.iter().any(|m| &m.id == id));
        self.count = self.snapshot.as_ref().map_or(0, |s| s.selected);
        for gesture in &self.queue {
            match &gesture.change {
                SelectionChange::All => {
                    self.visible = page
                        .rows
                        .iter()
                        .filter(|m| {
                            self.snapshot
                                .as_ref()
                                .is_none_or(|s| s.positions.contains_key(&m.id))
                        })
                        .map(|m| m.id.clone())
                        .collect();
                    self.count = self.snapshot.as_ref().map_or(page.total, |s| s.total);
                }
                SelectionChange::Clear => {
                    self.visible.clear();
                    self.count = 0;
                }
                SelectionChange::Set {
                    id,
                    selected,
                    clear_others,
                } => {
                    if *clear_others {
                        self.visible.clear();
                        self.count = 0;
                    }
                    let changed = if *selected {
                        self.visible.insert(id.clone())
                    } else {
                        self.visible.remove(id)
                    };
                    if changed {
                        self.count = if *selected {
                            self.count.saturating_add(1)
                        } else {
                            self.count.saturating_sub(1)
                        };
                    }
                }
                SelectionChange::Range { additive, .. } => {
                    if !additive {
                        self.visible.clear();
                    }
                    if let Some((from, to)) = gesture.range {
                        if !additive {
                            self.count = (to - from + 1) as usize;
                        }
                        for (index, mail) in page.rows.iter().enumerate() {
                            let position = (offset + index) as u64;
                            if (from..=to).contains(&position) {
                                let added = self.visible.insert(mail.id.clone());
                                if *additive && added {
                                    self.count = self.count.saturating_add(1);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
impl App {
    fn selection_ids(&self) -> Vec<String> {
        self.page.rows.iter().map(|m| m.id.clone()).collect()
    }
    pub(super) fn reconcile_selection_scope(&mut self) {
        if self
            .mail_selection
            .scope
            .as_ref()
            .is_some_and(|q| q != &scope(&self.query))
        {
            self.mail_selection.reset();
        }
        self.pump_selection();
    }
    pub(super) fn selection_page_changed(&mut self) {
        self.mail_selection.project(&self.page, self.query.offset);
        self.pump_selection();
    }
    pub(super) fn clear_selected_mail(&mut self) {
        if self.mail_selection.mode {
            self.mail_selection.anchor = None;
            self.queue_selection(SelectionChange::Clear);
        }
    }
    pub(super) fn clear_mail_selection(&mut self) {
        self.mail_selection.reset();
        self.pump_selection();
    }
    fn selection_position(&self, id: &str) -> Option<u64> {
        self.page
            .rows
            .iter()
            .position(|m| m.id == id)
            .map(|i| (self.query.offset + i) as u64)
            .or_else(|| {
                self.mail_selection
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.positions.get(id))
                    .copied()
            })
    }
    fn queue_selection(&mut self, change: SelectionChange) {
        if self.mail_selection.queue.len() >= CHANNEL_CAPACITY {
            self.notice("Selection is catching up. Try that click again.", true);
            return;
        }
        let range = if let SelectionChange::Range { anchor, target, .. } = &change {
            let start = self.selection_position(anchor).or_else(|| {
                self.mail_selection
                    .anchor
                    .as_ref()
                    .filter(|(id, _)| id == anchor)
                    .map(|(_, pos)| *pos)
            });
            start
                .zip(self.selection_position(target))
                .map(|(a, b)| (a.min(b), a.max(b)))
        } else {
            None
        };
        self.mail_selection
            .queue
            .push_back(Gesture { change, range });
        self.mail_selection.project(&self.page, self.query.offset);
        self.pump_selection();
    }
    pub(super) fn toggle_selection_mode(&mut self) -> Task<Message> {
        self.focused_input = None;
        self.pending_focus = None;
        self.sidebar_focus = false;
        self.list_focus = true;
        if self.mail_selection.mode {
            self.clear_mail_selection();
        } else {
            self.mail_selection.start(&self.query);
            self.pump_selection();
        }
        widget::operation::focus("unfocused")
    }
    pub(super) fn select_all_mail(&mut self) -> Task<Message> {
        if self.tab != Tab::Mail || self.full_reader || self.sidebar_focus || !self.list_focus {
            return Task::none();
        }
        // Another explicit Select All includes new arrivals; an existing
        // selection itself never grows silently during a background refresh.
        let anchor = self.mail_selection.anchor.clone().or_else(|| {
            self.selected
                .as_ref()
                .and_then(|id| self.selection_position(id).map(|p| (id.clone(), p)))
        });
        self.mail_selection.start(&self.query);
        self.mail_selection.anchor = anchor;
        self.queue_selection(SelectionChange::All);
        widget::operation::focus("unfocused")
    }
    pub(super) fn checkbox_mail(&mut self, id: String) -> Task<Message> {
        if !self.mail_selection.mode {
            self.mail_selection.start(&self.query);
        }
        self.focused_input = None;
        self.pending_focus = None;
        self.sidebar_focus = false;
        self.list_focus = true;
        self.last_click = None;
        let selected = !self.mail_selection.visible.contains(&id);
        self.mail_selection.anchor = self
            .selection_position(&id)
            .map(|position| (id.clone(), position));
        self.queue_selection(SelectionChange::Set {
            id,
            selected,
            clear_others: false,
        });
        widget::operation::focus("unfocused")
    }
    pub(super) fn click_select_mail(
        &mut self,
        id: String,
        modifiers: keyboard::Modifiers,
    ) -> Task<Message> {
        let toggle = modifiers.control() || modifiers.command();
        let range = modifiers.shift();
        self.focused_input = None;
        self.pending_focus = None;
        self.sidebar_focus = false;
        self.list_focus = true;
        if toggle || range {
            self.last_click = None;
            if !self.mail_selection.mode {
                self.mail_selection.start(&self.query);
                if let Some(previous) = self.selected.clone() {
                    self.mail_selection.anchor = self
                        .selection_position(&previous)
                        .map(|position| (previous.clone(), position));
                    self.queue_selection(SelectionChange::Set {
                        id: previous,
                        selected: true,
                        clear_others: false,
                    });
                }
            }
            if range && let Some((anchor, _)) = self.mail_selection.anchor.clone() {
                self.queue_selection(SelectionChange::Range {
                    anchor,
                    target: id,
                    additive: toggle,
                });
            } else {
                return self.checkbox_mail(id);
            }
            return widget::operation::focus("unfocused");
        }
        if self.mail_selection.mode {
            self.mail_selection.anchor = self
                .selection_position(&id)
                .map(|position| (id.clone(), position));
            self.queue_selection(SelectionChange::Set {
                id: id.clone(),
                selected: true,
                clear_others: true,
            });
        }
        self.handle(Message::Select(id))
    }
    pub(super) fn pump_selection(&mut self) {
        if self.mail_selection.inflight.is_some()
            || self
                .mail_selection
                .retry
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        let visible = self.selection_ids();
        let state = &self.mail_selection;
        let request = if let Some(id) = state.backend.filter(|id| Some(*id) != state.token) {
            Some(Request::Release(id))
        } else if let Some(id) = state.token {
            if let Some(snapshot) = &state.snapshot {
                if let Some(gesture) = state.queue.front() {
                    Some(Request::Change(
                        id,
                        snapshot.revision,
                        gesture.change.clone(),
                    ))
                } else if state.observed != visible {
                    Some(Request::Observe(id))
                } else {
                    None
                }
            } else {
                Some(Request::Capture(id, state.scope.clone().unwrap()))
            }
        } else {
            None
        };
        let Some(request) = request else {
            return;
        };
        let serial = self.mail_selection.serial + 1;
        if let Some(tx) = &self.tx
            && tx
                .try_send(Command::Selection(serial, request.clone(), visible.clone()))
                .is_ok()
        {
            self.mail_selection.serial = serial;
            if let Request::Capture(id, _) = request {
                self.mail_selection.backend = Some(id);
            }
            self.mail_selection.observed = visible;
            self.mail_selection.inflight = Some((serial, request));
            self.mail_selection.retry = None;
        } else {
            if self.mail_selection.retry.is_none() {
                self.notice(
                    "Selection is waiting for the background queue. Retrying…",
                    true,
                );
            }
            self.mail_selection.retry = Some(Instant::now());
        }
    }
    pub(super) fn selection_finished(
        &mut self,
        serial: u64,
        result: Result<Option<Arc<SelectionSnapshot>>, String>,
    ) {
        if self
            .mail_selection
            .inflight
            .as_ref()
            .is_none_or(|(expected, _)| *expected != serial)
        {
            return;
        }
        let (_, request) = self.mail_selection.inflight.take().unwrap();
        let active = self.mail_selection.token == Some(request.id());
        match result {
            Ok(snapshot) => {
                if matches!(request, Request::Release(_)) {
                    self.mail_selection.backend = None;
                } else if active {
                    if matches!(request, Request::Change(..)) {
                        self.mail_selection.queue.pop_front();
                    }
                    if let Some((anchor, position)) = &mut self.mail_selection.anchor
                        && let Some(observed) =
                            snapshot.as_ref().and_then(|s| s.positions.get(anchor))
                    {
                        *position = *observed;
                    }
                    self.mail_selection.snapshot = snapshot;
                }
            }
            Err(error) => {
                if active && matches!(request, Request::Observe(_)) {
                    self.mail_selection.observed.clear();
                    self.mail_selection.retry = Some(Instant::now());
                    self.notice(
                        format!("Could not refresh the selection. Retrying… {error}"),
                        true,
                    );
                    self.mail_selection.project(&self.page, self.query.offset);
                    return;
                }
                if active && matches!(request, Request::Change(..)) {
                    // Store changes are atomic. Reject only this gesture, keep
                    // confirmed membership and process subsequent native input.
                    self.mail_selection.queue.pop_front();
                    self.notice(format!("Could not update this selection. {error}"), true);
                    self.mail_selection.project(&self.page, self.query.offset);
                    self.pump_selection();
                    return;
                }
                if matches!(request, Request::Release(_)) {
                    self.notice("Could not reset the selection. Retrying…", true);
                }
                if active {
                    self.mail_selection.reset();
                    self.notice(
                        format!(
                            "Selection could not be updated. Select the messages again. {error}"
                        ),
                        true,
                    );
                }
                self.mail_selection.retry = Some(Instant::now());
            }
        }
        self.mail_selection.project(&self.page, self.query.offset);
        self.pump_selection();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    async fn fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>) {
        let store = Store::memory().unwrap();
        let mut mail = Vec::new();
        for i in 0..125 {
            mail.push(
                parse_mail(
                    "fixture",
                    &i.to_string(),
                    "INBOX",
                    format!("From: fixture@example.test\r\nSubject: {i:03}\r\n\r\nBody")
                        .into_bytes(),
                    true,
                    false,
                )
                .unwrap(),
            );
        }
        store.upsert(mail).await.unwrap();
        let (sender, receiver) = engine::CommandSender::selection_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.sort = MailSort::Subject;
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        (app, store, receiver)
    }
    async fn reply(
        app: &mut App,
        store: &Store,
        commands: &mut tokio::sync::mpsc::Receiver<Command>,
    ) {
        let Command::Selection(serial, request, visible) = commands.try_recv().unwrap() else {
            panic!("Expected a selection request");
        };
        let result = match request {
            Request::Capture(id, query) => store
                .capture_selection(id, 0, query, false, visible)
                .await
                .map(Some),
            Request::Change(id, revision, change) => store
                .change_selection(id, revision, change, visible)
                .await
                .map(Some),
            Request::Observe(id) => store.selection_snapshot(id, visible).await.map(Some),
            Request::Release(id) => store.release_selection(id).await.map(|()| None),
        }
        .map(|s| s.map(Arc::new))
        .map_err(|e| e.to_string());
        app.selection_finished(serial, result);
    }
    #[tokio::test]
    async fn passive_selection_observation_does_not_disable_dragging_confirmed_choices() {
        let (mut app, store, mut commands) = fixture().await;
        let first = app.page.rows[0].id.clone();
        let _ = app.checkbox_mail(first);
        assert!(!app.mail_selection.ready_for_drag());
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert!(app.mail_selection.ready_for_drag());
        store
            .upsert(vec![
                parse_mail(
                    "fixture",
                    "arrival",
                    "INBOX",
                    b"Subject: 000A\r\n\r\nArrival".to_vec(),
                    true,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        assert!(app.mail_selection.busy());
        assert!(app.mail_selection.ready_for_drag());
        reply(&mut app, &store, &mut commands).await;
        let _ = app.checkbox_mail(app.page.rows[1].id.clone());
        assert!(
            !app.mail_selection.ready_for_drag(),
            "Unacknowledged membership edits still need a reviewable snapshot"
        );
    }
    #[tokio::test]
    async fn gestures_are_immediate_ordered_and_only_hold_one_metadata_page() {
        let (mut app, store, mut commands) = fixture().await;
        let first = app.page.rows[0].id.clone();
        let _ = app.checkbox_mail(first.clone());
        assert!(app.mail_selection.visible.contains(&first));
        assert_eq!(app.mail_selection.count, 1);
        let _ = app.checkbox_mail(app.page.rows[1].id.clone());
        assert_eq!(app.mail_selection.count, 2);
        assert!(
            app.mail_actions.read_candidate.is_none(),
            "Checkboxes must not count as reading"
        );
        reply(&mut app, &store, &mut commands).await;
        reply(&mut app, &store, &mut commands).await;
        reply(&mut app, &store, &mut commands).await;
        assert!(!app.mail_selection.busy());
        let _ = app.select_all_mail();
        assert_eq!(app.mail_selection.count, 125);
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        app.query.offset = 50;
        app.reconcile_selection_scope();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        reply(&mut app, &store, &mut commands).await;
        assert_eq!(app.mail_selection.count, 125);
        assert_eq!(app.mail_selection.visible.len(), 50);
        let _ = app.checkbox_mail(app.page.rows[4].id.clone());
        assert_eq!(app.mail_selection.count, 124);
        reply(&mut app, &store, &mut commands).await;
        assert_eq!(app.mail_selection.count, 124);
        assert_eq!(app.mail_selection.visible.len(), 49);
        assert_eq!(
            app.mail_selection
                .snapshot
                .as_ref()
                .unwrap()
                .positions
                .len(),
            50
        );
    }
    #[tokio::test]
    async fn cancellation_releases_old_scope_and_late_results_cannot_revive_it() {
        let (mut app, store, mut commands) = fixture().await;
        let _ = app.select_all_mail();
        let old = app.mail_selection.token.unwrap();
        // Several scopes change while a capture is still in flight. Only its
        // one database snapshot needs cleanup; unsent intentions replace freely.
        for _ in 0..100 {
            app.clear_mail_selection();
            let _ = app.select_all_mail();
        }
        let latest = app.mail_selection.token.unwrap();
        assert_ne!(old, latest);
        reply(&mut app, &store, &mut commands).await; // obsolete capture
        assert_eq!(app.mail_selection.token, Some(latest));
        assert!(app.mail_selection.snapshot.is_none());
        reply(&mut app, &store, &mut commands).await; // release old
        assert!(store.selection_snapshot(old, vec![]).await.is_err());
        reply(&mut app, &store, &mut commands).await; // current capture
        reply(&mut app, &store, &mut commands).await; // all
        app.selection_finished(1, Err("Obsolete failure".into()));
        assert_eq!(app.mail_selection.count, 125);
        assert!(app.mail_selection.ready());
        app.query.search = "another query".into();
        app.reconcile_selection_scope();
        assert!(!app.mail_selection.mode);
        reply(&mut app, &store, &mut commands).await;
        assert!(store.selection_snapshot(latest, vec![]).await.is_err());
    }
    #[tokio::test]
    async fn ranges_span_pages_without_marking_read_and_backpressure_keeps_exact_acknowledgments() {
        let (mut app, store, mut commands) = fixture().await;
        let _ = app.checkbox_mail(app.page.rows[1].id.clone());
        reply(&mut app, &store, &mut commands).await;
        reply(&mut app, &store, &mut commands).await;
        app.query.offset = 50;
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        reply(&mut app, &store, &mut commands).await;
        let _ = app.click_select_mail(app.page.rows[2].id.clone(), keyboard::Modifiers::SHIFT);
        assert_eq!(app.mail_selection.count, 52);
        reply(&mut app, &store, &mut commands).await;
        assert_eq!(app.mail_selection.count, 52);
        assert!(app.mail_actions.read_candidate.is_none());
        for _ in 0..40 {
            let _ = app.checkbox_mail(app.page.rows[3].id.clone());
        }
        assert_eq!(app.mail_selection.queue.len(), CHANNEL_CAPACITY);
        assert!(app.notice.as_ref().unwrap().1);
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert_eq!(app.mail_selection.count, 52);
        let token = app.mail_selection.token.unwrap();
        assert_eq!(
            store
                .selection_snapshot(token, vec![])
                .await
                .unwrap()
                .selected,
            52
        );
    }
    #[tokio::test]
    async fn arrivals_require_an_explicit_new_select_all_and_search_clears_scope_immediately() {
        let (mut app, store, mut commands) = fixture().await;
        let _ = app.select_all_mail();
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        store
            .upsert(vec![
                parse_mail(
                    "fixture",
                    "new",
                    "INBOX",
                    b"From: new@example.test\r\nSubject: New arrival\r\n\r\nNew".to_vec(),
                    true,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        assert_eq!(app.mail_selection.count, 125);
        let _ = app.select_all_mail();
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert_eq!(app.mail_selection.count, 126);
        let _ = app.handle(Message::Query("new".into()));
        assert!(
            !app.mail_selection.mode,
            "Clear before the search debounce, not after a new page"
        );
    }
    #[tokio::test]
    async fn select_all_is_scoped_to_list_focus_and_rejection_keeps_confirmed_membership() {
        let (mut app, store, mut commands) = fixture().await;
        app.list_focus = false;
        let _ = app.select_all_mail();
        assert!(!app.mail_selection.mode);
        app.list_focus = true;
        let _ = app.select_all_mail();
        reply(&mut app, &store, &mut commands).await;
        let (serial, _) = app.mail_selection.inflight.clone().unwrap();
        app.selection_finished(serial, Err("Fixture write rejected".into()));
        assert!(app.mail_selection.mode);
        assert_eq!(app.mail_selection.count, 0);
        assert!(app.mail_selection.visible.is_empty());
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .contains("Fixture write rejected")
        );
    }

    #[tokio::test]
    async fn selecting_an_arrival_and_rejecting_a_vanished_target_keeps_prior_choices() {
        let (mut app, store, mut commands) = fixture().await;
        let first = app.page.rows[0].id.clone();
        let _ = app.checkbox_mail(first.clone());
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        let arrival = parse_mail(
            "fixture",
            "arrival",
            "INBOX",
            b"Subject: 000A\r\n\r\nNew mail".to_vec(),
            true,
            false,
        )
        .unwrap();
        let new_id = arrival.summary.id.clone();
        store.upsert(vec![arrival]).await.unwrap();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        let Command::Selection(serial, Request::Observe(_), _) = commands.try_recv().unwrap()
        else {
            panic!("The new page must observe existing selection membership");
        };
        app.selection_finished(serial, Err("Temporary read failure".into()));
        assert!(app.mail_selection.mode);
        assert_eq!(app.mail_selection.count, 1);
        assert!(
            commands.try_recv().is_err(),
            "Retry must not spin on a failed cache read"
        );
        app.mail_selection.retry = Some(Instant::now() - std::time::Duration::from_secs(2));
        app.pump_selection();
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert_eq!(
            app.mail_selection.count, 1,
            "Passive arrival must not select itself"
        );
        let _ = app.checkbox_mail(new_id.clone());
        assert_eq!(
            app.mail_selection.count, 2,
            "Explicit clicks project immediately"
        );
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert_eq!(
            app.mail_selection.visible,
            [first.clone(), new_id.clone()].into()
        );
        assert!(app.mail_actions.read_candidate.is_none());

        let transient = parse_mail(
            "fixture",
            "transient",
            "INBOX",
            b"Subject: 000B\r\n\r\nLeaving mail".to_vec(),
            true,
            false,
        )
        .unwrap();
        let transient_id = transient.summary.id.clone();
        store.upsert(vec![transient]).await.unwrap();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        let _ = app.checkbox_mail(transient_id.clone());
        store.remove(transient_id).await.unwrap();
        while app.mail_selection.busy() {
            reply(&mut app, &store, &mut commands).await;
        }
        assert!(app.mail_selection.mode);
        assert_eq!(app.mail_selection.count, 2);
        assert_eq!(app.mail_selection.visible, [first, new_id].into());
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .contains("previous selection is kept")
        );
    }
}
