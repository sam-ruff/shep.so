use super::*;
use crate::notifications::{Arrival, Delivery, Event};
use iced::widget::{checkbox, column, text};

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Popups,
    Sound,
    Details,
}

#[derive(Debug, Clone)]
pub enum Message {
    Backend(Event),
    Preference(Control, bool),
    Test,
}

#[derive(Default)]
pub(super) struct State {
    request: crate::notifications::State,
    sender: Option<tokio::sync::watch::Sender<crate::notifications::State>>,
    system: Option<crate::notifications::SystemSender>,
    testing: Option<u64>,
    error: Option<String>,
    sent: u64,
    last: Option<Delivery>,
}

impl App {
    pub(super) fn saving_notification(
        &self,
        cancellation: tokio::sync::watch::Receiver<()>,
    ) -> impl Future<Output = Result<(), String>> + use<> {
        let sender = self.notifications.system.clone();
        async move {
            sender
                .ok_or_else(|| "The desktop notification worker is not ready".to_owned())?
                .saving_notification(cancellation)
                .await
        }
    }

    pub(super) fn notification_arrived(&mut self, arrival: Arc<Arrival>) {
        if !self
            .workspace
            .accounts
            .iter()
            .any(|a| a.id == arrival.account)
        {
            return;
        }
        self.notifications.request.total = self.notifications.request.total.saturating_add(1);
        self.notifications.request.latest = Some(arrival);
    }

    pub(super) fn update_notification_settings(&mut self) {
        self.notifications.request.settings = self.preferences.notifications;
        // Do not present details from a connection removed during the burst wait.
        if self
            .notifications
            .request
            .latest
            .as_ref()
            .is_some_and(|arrival| {
                !arrival.account.is_empty()
                    && !self
                        .workspace
                        .accounts
                        .iter()
                        .any(|a| a.id == arrival.account)
            })
        {
            self.notifications.request.latest = None;
        }
        if let Some(sender) = &self.notifications.sender {
            sender.send_if_modified(|state| {
                if *state == self.notifications.request {
                    return false;
                }
                *state = self.notifications.request.clone();
                true
            });
        }
    }

    pub(super) fn handle_notification(&mut self, message: Message) {
        match message {
            Message::Preference(control, enabled) => {
                match control {
                    Control::Popups => self.preferences.notifications.popups = enabled,
                    Control::Sound => self.preferences.notifications.sound = enabled,
                    Control::Details => self.preferences.notifications.show_details = enabled,
                }
                self.notifications.error = None;
                self.save_preferences();
            }
            Message::Backend(Event::Ready(sender, system)) => {
                self.notifications.sender = Some(sender);
                self.notifications.system = Some(system);
            }
            Message::Backend(Event::Skipped(through)) => {
                if self
                    .notifications
                    .testing
                    .is_some_and(|serial| serial <= through)
                {
                    self.notifications.testing = None;
                }
            }
            Message::Backend(Event::Sent(delivery)) => {
                self.notifications.sent += 1;
                if self
                    .notifications
                    .testing
                    .is_some_and(|serial| serial <= delivery.through)
                {
                    self.notifications.testing = None;
                    self.notice("Test notification requested.", false);
                }
                self.notifications.last = Some(delivery);
                self.notifications.error = None;
            }
            Message::Backend(Event::Failed(delivery, error)) => {
                if self
                    .notifications
                    .testing
                    .is_some_and(|serial| serial <= delivery.through)
                {
                    self.notifications.testing = None;
                }
                if delivery.popups == self.preferences.notifications.popups
                    && delivery.sound == self.preferences.notifications.sound
                {
                    if self.notifications.error.as_ref() != Some(&error) {
                        self.notice(error.clone(), true);
                    }
                    self.notifications.error = Some(error);
                }
            }
            Message::Test => {
                if self.notifications.testing.is_some()
                    || !(self.preferences.notifications.popups
                        || self.preferences.notifications.sound)
                {
                    return;
                }
                self.notifications.error = None;
                self.notifications.request.total =
                    self.notifications.request.total.saturating_add(1);
                self.notifications.testing = Some(self.notifications.request.total);
                self.notifications.request.latest = Some(Arc::new(Arrival {
                    account: String::new(),
                    message: String::new(),
                    sender: "Shep".into(),
                    subject: "Your notification settings are working.".into(),
                }));
            }
        }
    }

    pub(super) fn notification_settings(&self) -> Element<'_, super::Message> {
        let mut content = column![
            checkbox(self.preferences.notifications.popups)
                .label("Show new-mail popups")
                .on_toggle(|value| super::Message::Notification(Message::Preference(
                    Control::Popups,
                    value
                ))),
            checkbox(self.preferences.notifications.sound)
                .label("Play a sound for new mail")
                .on_toggle(|value| super::Message::Notification(Message::Preference(
                    Control::Sound,
                    value
                ))),
            checkbox(self.preferences.notifications.show_details)
                .label("Show sender and subject in popups")
                .on_toggle(|value| super::Message::Notification(Message::Preference(
                    Control::Details,
                    value
                ))),
            widget::button(
                text(if self.notifications.testing.is_some() {
                    "Sending test…"
                } else {
                    "Test notification"
                })
                .size(13)
            )
            .padding([10, 14])
            .style(outline)
            .on_press_maybe(
                (self.notifications.testing.is_none()
                    && (self.preferences.notifications.popups
                        || self.preferences.notifications.sound))
                    .then_some(super::Message::Notification(Message::Test))
            ),
        ]
        .spacing(18);
        if let Some(error) = &self.notifications.error {
            content = content.push(text(error).size(12).style(widget::text::danger));
        }
        self.settings_card(
            "Notifications",
            "For new unread messages in your Inbox.",
            content.into(),
        )
    }

    #[cfg(feature = "test-support")]
    pub(super) fn notification_observation(&self) -> serde_json::Value {
        serde_json::json!({
            "settings":self.preferences.notifications,
            "saved":self.workspace.preferences.notifications,
            "requested":self.notifications.request.total,
            "sent":self.notifications.sent,
            "last":self.notifications.last,
            "testing":self.notifications.testing.is_some(),
            "error":self.notifications.error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_test_muted_during_burst_finishes_without_a_false_success() {
        let (mut app, _) = App::new();
        app.handle_notification(Message::Test);
        assert_eq!(app.notifications.testing, Some(1));
        app.handle_notification(Message::Preference(Control::Popups, false));
        app.handle_notification(Message::Preference(Control::Sound, false));
        app.handle_notification(Message::Backend(Event::Skipped(1)));
        assert!(app.notifications.testing.is_none());
        assert_eq!(app.notifications.sent, 0);
        app.handle_notification(Message::Test);
        assert_eq!(app.notifications.request.total, 1);
    }

    #[test]
    fn notification_errors_remain_actionable_and_ignore_obsolete_muted_requests() {
        let (mut app, _) = App::new();
        let delivery = Delivery {
            through: 1,
            count: 1,
            popups: true,
            sound: true,
            title: "Fixture".into(),
            body: "Fixture".into(),
        };
        app.handle_notification(Message::Test);
        app.handle_notification(Message::Backend(Event::Failed(
            delivery.clone(),
            "Check desktop permissions.".into(),
        )));
        assert!(app.notifications.testing.is_none());
        assert_eq!(
            app.notifications.error.as_deref(),
            Some("Check desktop permissions.")
        );
        assert!(app.notice.as_ref().unwrap().1);
        app.handle_notification(Message::Preference(Control::Sound, false));
        app.handle_notification(Message::Backend(Event::Failed(
            delivery,
            "Obsolete failure".into(),
        )));
        assert!(app.notifications.error.is_none());
        assert!(!app.preferences.notifications.sound);
    }

    #[test]
    fn notification_for_removed_account_is_not_presented() {
        let (mut app, _) = App::new();
        let arrival = Arc::new(Arrival {
            account: "removed".into(),
            message: "one".into(),
            sender: "Fixture".into(),
            subject: "Fixture".into(),
        });
        app.notification_arrived(arrival.clone());
        assert_eq!(app.notifications.request.total, 0);
        app.notifications.request.latest = Some(arrival);
        app.update_notification_settings();
        assert!(app.notifications.request.latest.is_none());
    }
}
