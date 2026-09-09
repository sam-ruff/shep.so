//! Keep the reader adjacent to a removed row, independent of provider latency.
use super::*;

pub(super) struct Neighbors {
    position: usize,
    next: Option<String>,
    previous: Option<String>,
    more: bool,
}

pub(in crate::ui) struct Follow {
    query: MailQuery,
    displayed: Option<String>,
    removed: String,
    position: usize,
    reveal: bool,
}

impl App {
    pub(super) fn removal_neighbors(&self, id: &str) -> Option<Neighbors> {
        if self.selected.as_deref() != Some(id) && self.reader_id() != Some(id) {
            return None;
        }
        let position = self.page.rows.iter().position(|mail| mail.id == id)?;
        Some(Neighbors {
            position,
            next: self.page.rows.get(position + 1).map(|mail| mail.id.clone()),
            previous: position
                .checked_sub(1)
                .and_then(|index| self.page.rows.get(index))
                .map(|mail| mail.id.clone()),
            more: self.query.offset + self.page.rows.len() < self.page.total,
        })
    }

    pub(super) fn select_after_removal(&mut self, id: &str, neighbors: Option<Neighbors>) {
        let Some(neighbors) = neighbors else {
            return;
        };
        if self.page.rows.iter().any(|mail| mail.id == id) {
            // A cross-folder search can still include the moved message.
            return;
        }
        let next = neighbors
            .next
            .as_ref()
            .filter(|next| self.page.rows.iter().any(|mail| &mail.id == *next))
            .or_else(|| {
                neighbors
                    .previous
                    .as_ref()
                    .filter(|previous| self.page.rows.iter().any(|mail| &mail.id == *previous))
            })
            .cloned();
        self.selected = None;
        self.detail = None;
        self.conversation = Default::default();
        if let Some(next) = next {
            self.select(next);
        }

        // At a page boundary the next row may be outside the metadata page.
        // Ask the normal bounded reader queue for it while the move is pending;
        // newer explicit navigation cancels this follow-up through `select`.
        let preceding_page = self.page.rows.is_empty() && self.query.offset > 0;
        if preceding_page || (neighbors.next.is_none() && neighbors.more) {
            let position = if preceding_page {
                self.query.offset = self.query.offset.saturating_sub(PAGE_SIZE);
                PAGE_SIZE - 1
            } else {
                neighbors.position
            };
            self.request_page();
            self.mail_actions.follow = Some(Follow {
                query: self.query.clone(),
                displayed: self.selected.clone(),
                removed: id.into(),
                position,
                reveal: preceding_page,
            });
        }
    }

    pub(in crate::ui) fn reveal_selected_mail(&mut self) -> Task<Message> {
        let Some(index) = self
            .selected
            .as_ref()
            .and_then(|id| self.page.rows.iter().position(|mail| &mail.id == id))
        else {
            return Task::none();
        };
        let viewport =
            (self.size.height / (self.preferences.interface_scale as f32 / 100.) - 220.).max(104.);
        let top = index as f32 * 104.;
        self.inbox_scroll = if top < self.inbox_scroll {
            top
        } else if top + 104. > self.inbox_scroll + viewport {
            top + 104. - viewport
        } else {
            self.inbox_scroll
        };
        widget::operation::scroll_to(
            "inbox-list",
            widget::scrollable::AbsoluteOffset {
                x: 0.,
                y: self.inbox_scroll,
            },
        )
    }

    pub(in crate::ui) fn finish_removal_selection(&mut self) -> bool {
        let Some(follow) = self.mail_actions.follow.take() else {
            return false;
        };
        if follow.query != self.query
            || follow.displayed != self.selected
            || self.page.rows.iter().any(|mail| mail.id == follow.removed)
        {
            return false;
        }
        if self.page.rows.get(follow.position).is_none()
            && self.query.offset + follow.position < self.page.total
        {
            // A projected page may be short until its refill arrives. Its total
            // still promises the successor; do not finalize a previous-row fallback.
            self.mail_actions.follow = Some(follow);
            return false;
        }
        if let Some(mail) = self
            .page
            .rows
            .get(follow.position)
            .or_else(|| self.page.rows.last())
        {
            self.select(mail.id.clone());
            return follow.reveal;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture(count: usize) -> (App, tokio::sync::mpsc::Receiver<Command>) {
        let (mut app, commands, original) = super::super::tests::fixture().await;
        let rows = (0..count)
            .map(|index| {
                let mut mail = original.summary.clone();
                mail.id = format!("message-{index}");
                mail.remote_id = index.to_string();
                mail.subject = format!("Displayed row {index}");
                mail.unread = false;
                mail
            })
            .collect::<Vec<_>>();
        app.set_mail_page(Arc::new(MailPage {
            total: count,
            rows,
            ..Default::default()
        }));
        app.detail = None;
        app.last_list_query = app.query.clone();
        (app, commands)
    }

    #[tokio::test]
    async fn deletion_follows_displayed_neighbors_without_resetting_scroll() {
        let (mut app, _commands) = fixture(6).await;
        app.selected = Some("message-3".into());
        app.inbox_scroll = 120.;
        for expected in ["message-4", "message-5", "message-2"] {
            let mail = app.action_mail().unwrap().clone();
            app.move_mail(mail, "Trash".into());
            assert_eq!(app.selected.as_deref(), Some(expected));
            assert_eq!(app.inbox_scroll, 120.);
        }
        // A late provider page still containing all original rows is projected;
        // it cannot return to the removed row or select the first message.
        let original = app.mail_actions.base_page.clone();
        let _ = app.handle(Message::Backend(Event::Page(
            app.generation,
            original,
            false,
        )));
        assert_eq!(app.selected.as_deref(), Some("message-2"));
        assert_eq!(app.inbox_scroll, 120.);
    }

    #[tokio::test]
    async fn rejection_restores_the_row_without_overriding_newer_selection() {
        let (mut app, mut commands) = fixture(6).await;
        app.selected = Some("message-3".into());
        app.inbox_scroll = 120.;
        app.move_mail(app.page.rows[3].clone(), "Trash".into());
        assert_eq!(app.selected.as_deref(), Some("message-4"));
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!("expected move");
        };
        app.select("message-1".into());
        let _ = app.move_finished(request, mail, folder, Err("Fixture rejection".into()));
        assert_eq!(app.selected.as_deref(), Some("message-1"));
        assert_eq!(app.inbox_scroll, 120.);
        assert!(app.page.rows.iter().any(|mail| mail.id == "message-3"));
        assert!(app.notice.as_ref().unwrap().1);
    }

    #[tokio::test]
    async fn deletion_uses_displayed_order_for_filtered_sorted_rows() {
        let (mut app, _commands) = fixture(6).await;
        app.query.sort = MailSort::Oldest;
        app.query.starred_only = true;
        let mut page = (*app.mail_actions.base_page).clone();
        page.rows.reverse();
        for mail in &mut page.rows {
            mail.starred = true;
        }
        app.set_mail_page(Arc::new(page));
        app.last_list_query = app.query.clone();
        app.selected = Some("message-3".into());
        app.move_mail(app.page.rows[2].clone(), "Trash".into());
        assert_eq!(app.selected.as_deref(), Some("message-2"));
    }

    #[tokio::test]
    async fn deleting_the_only_message_clears_the_reader() {
        let (mut app, _commands) = fixture(1).await;
        app.selected = Some("message-0".into());
        let mail = app.page.rows[0].clone();
        app.move_mail(mail, "Trash".into());
        assert!(app.selected.is_none());
        assert!(app.detail.is_none());
        assert!(app.page.rows.is_empty());
    }

    #[tokio::test]
    async fn page_boundary_follow_up_does_not_override_newer_navigation() {
        for navigate in [false, true] {
            let (mut app, _commands) = fixture(3).await;
            Arc::make_mut(&mut app.mail_actions.base_page).total = 4;
            app.project_mail_flags();
            app.selected = Some("message-2".into());
            app.move_mail(app.page.rows[2].clone(), "Trash".into());
            assert_eq!(app.selected.as_deref(), Some("message-1"));
            assert!(app.mail_actions.follow.is_some());
            let short = app.mail_actions.base_page.clone();
            let _ = app.handle(Message::Backend(Event::Page(app.generation, short, false)));
            assert_eq!(app.page.rows.len(), 2);
            assert_eq!(app.page.total, 3);
            assert_eq!(app.selected.as_deref(), Some("message-1"));
            assert!(app.mail_actions.follow.is_some());
            if navigate {
                app.select("message-0".into());
                assert!(app.mail_actions.follow.is_none());
            }
            let mut page = (*app.mail_actions.base_page).clone();
            page.rows.retain(|mail| mail.id != "message-2");
            let mut following = page.rows[1].clone();
            following.id = "following-page".into();
            page.rows.push(following);
            let _ = app.handle(Message::Backend(Event::Page(
                app.generation,
                Arc::new(page),
                false,
            )));
            assert_eq!(
                app.selected.as_deref(),
                Some(if navigate {
                    "message-0"
                } else {
                    "following-page"
                })
            );
        }
    }

    #[tokio::test]
    async fn emptied_final_page_returns_to_previous_last_row() {
        let (mut app, _commands) = fixture(1).await;
        app.query.offset = PAGE_SIZE;
        app.last_list_query = app.query.clone();
        app.selected = Some("message-0".into());
        app.move_mail(app.page.rows[0].clone(), "Trash".into());
        assert_eq!(app.query.offset, 0);
        assert!(app.mail_actions.follow.is_some());
        let mut page = MailPage::default();
        for index in 0..PAGE_SIZE {
            let mut mail = app.mail_actions.base_page.rows[0].clone();
            mail.id = format!("previous-{index}");
            page.rows.push(mail);
        }
        page.total = PAGE_SIZE;
        let _ = app.handle(Message::Backend(Event::Page(
            app.generation,
            Arc::new(page),
            false,
        )));
        assert_eq!(app.selected.as_deref(), Some("previous-49"));
        assert!(app.inbox_scroll > 0.);
    }
}
