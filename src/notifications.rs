//! New-mail notification policy and native delivery. Provider/storage work
//! decides what arrived; native presentation stays independent of mail sync.
use futures::{SinkExt, Stream};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

mod native;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub popups: bool,
    pub sound: bool,
    pub show_details: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            popups: true,
            sound: true,
            show_details: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrival {
    pub account: String,
    pub message: String,
    pub sender: String,
    pub subject: String,
}
impl Arrival {
    pub(crate) fn from_mail(mail: &crate::model::Mail) -> Self {
        fn label(value: &str) -> String {
            value
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect()
        }
        Self {
            account: mail.account_id.clone(),
            message: mail.id.clone(),
            sender: label(&mail.sender),
            subject: label(&mail.subject),
        }
    }
}

/// A watch value coalesces bursts without dropping their count or queuing
/// unbounded message payloads when the desktop service is slow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub settings: Settings,
    pub total: u64,
    pub latest: Option<Arc<Arrival>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub through: u64,
    pub count: u64,
    pub popups: bool,
    pub sound: bool,
    pub title: String,
    pub body: String,
}
impl Delivery {
    fn prepare(state: &State, count: u64) -> Option<Self> {
        if count == 0 || !(state.settings.popups || state.settings.sound) {
            return None;
        }
        let latest = state.latest.as_ref()?;
        let (title, body) = if state.settings.show_details && count == 1 {
            (
                if latest.sender.is_empty() {
                    "New email".into()
                } else {
                    latest.sender.clone()
                },
                if latest.subject.is_empty() {
                    "(No subject)".into()
                } else {
                    latest.subject.clone()
                },
            )
        } else if count == 1 {
            (
                "New email".into(),
                "You have a new message in your Inbox.".into(),
            )
        } else {
            (
                format!("{count} new emails"),
                "Open your Inbox to read them.".into(),
            )
        };
        Some(Self {
            through: state.total,
            count,
            popups: state.settings.popups,
            sound: state.settings.sound,
            title,
            body,
        })
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Ready(watch::Sender<State>),
    Sent(Delivery),
    Failed(Delivery, String),
    Skipped(u64),
}

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(
        8,
        move |mut output: futures::channel::mpsc::Sender<Event>| async move {
            let (tx, rx) = watch::channel(State::default());
            if output.send(Event::Ready(tx)).await.is_err() {
                return;
            }
            let mut attempt = 0u64;
            drive(rx, output, move |delivery| {
                attempt = attempt.saturating_add(1);
                let _attempt = attempt;
                async move {
                    // The harness can observe its policy/requests but never notify or
                    // play sound on the person's real desktop from a fixture workspace.
                    if demo {
                        #[cfg(feature = "test-support")]
                        crate::test_support::notification_delivery(_attempt).await?;
                        Ok(())
                    } else {
                        native::deliver(delivery).await
                    }
                }
            })
            .await;
        },
    )
}

async fn drive<F, Fut>(
    mut input: watch::Receiver<State>,
    mut output: futures::channel::mpsc::Sender<Event>,
    mut deliver: F,
) where
    F: FnMut(Delivery) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut processed = 0;
    while input.changed().await.is_ok() {
        // A fixed burst window does not postpone notifications indefinitely
        // during continuous arrivals. Settings are sampled after that window.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let state = input.borrow_and_update().clone();
        let count = state.total.saturating_sub(processed);
        processed = state.total;
        let Some(delivery) = Delivery::prepare(&state, count) else {
            if count > 0 && output.send(Event::Skipped(state.total)).await.is_err() {
                break;
            }
            continue;
        };
        let event = match deliver(delivery.clone()).await {
            Ok(()) => Event::Sent(delivery),
            Err(error) => Event::Failed(delivery, format!("{error:#}")),
        };
        if output.send(event).await.is_err() {
            break;
        }
    }
}

pub(crate) async fn saving_notification() -> anyhow::Result<()> {
    native::deliver(Delivery {
        through: 0,
        count: 0,
        popups: true,
        sound: false,
        title: "Shep is finishing your changes".into(),
        body: "Shep will quit when your changes are saved. Open it from the tray to keep working."
            .into(),
    })
    .await
}
