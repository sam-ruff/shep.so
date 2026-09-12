//! New-mail notification policy and native delivery. Provider/storage work
//! decides what arrived; native presentation stays independent of mail sync.
use futures::{SinkExt, Stream};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(target_os = "linux")]
mod connection;
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
    Ready(watch::Sender<State>, SystemSender),
    Sent(Delivery),
    Failed(Delivery, String),
    Skipped(u64),
}

#[derive(Debug, Clone)]
pub struct SystemSender(mpsc::Sender<SystemRequest>);

#[derive(Debug)]
struct SystemRequest {
    delivery: Delivery,
    receipt: oneshot::Sender<Result<(), String>>,
    cancellation: watch::Receiver<()>,
}

impl SystemSender {
    pub(crate) async fn saving_notification(
        self,
        mut cancellation: watch::Receiver<()>,
    ) -> Result<(), String> {
        let (receipt, result) = oneshot::channel();
        let request = SystemRequest {
            delivery: saving_delivery(),
            receipt,
            cancellation: cancellation.clone(),
        };
        let delivery = async move {
            self.0
                .send(request)
                .await
                .map_err(|_| "The desktop notification worker stopped".to_owned())?;
            result
                .await
                .map_err(|_| "The desktop notification worker stopped".to_owned())?
        };
        tokio::time::timeout(Duration::from_secs(10), async move {
            tokio::select! {
                biased;
                _ = cancellation.changed() => Err("Saving notification cancelled".to_owned()),
                result = delivery => result,
            }
        })
        .await
        .map_err(|_| "The desktop notification worker did not respond".to_owned())?
    }
}

#[async_trait::async_trait]
trait Backend: Send {
    async fn deliver(&mut self, delivery: Delivery) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl<F, Fut> Backend for F
where
    F: FnMut(Delivery) -> Fut + Send,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send,
{
    async fn deliver(&mut self, delivery: Delivery) -> anyhow::Result<()> {
        self(delivery).await
    }
}

struct DesktopBackend {
    demo: bool,
    attempt: u64,
    #[cfg(target_os = "linux")]
    client: connection::Client<connection::SessionBus>,
}

#[async_trait::async_trait]
impl Backend for DesktopBackend {
    async fn deliver(&mut self, delivery: Delivery) -> anyhow::Result<()> {
        self.attempt = self.attempt.saturating_add(1);
        if self.demo {
            let saving = delivery.through == 0 && crate::desktop_tray::fixture_permitted();
            // Ordinary previews never use a desktop service or an audio device.
            #[cfg(feature = "test-support")]
            if !saving && !crate::test_support::native_notifications_permitted()? {
                return crate::test_support::notification_delivery(self.attempt).await;
            }
            #[cfg(not(feature = "test-support"))]
            if !saving {
                return Ok(());
            }
        }
        #[cfg(target_os = "linux")]
        return self.client.deliver(delivery).await;
        #[cfg(not(target_os = "linux"))]
        native::deliver(delivery).await
    }
}

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(
        8,
        move |mut output: futures::channel::mpsc::Sender<Event>| async move {
            let (tx, rx) = watch::channel(State::default());
            let (system, requests) = mpsc::channel(1);
            if output
                .send(Event::Ready(tx, SystemSender(system)))
                .await
                .is_err()
            {
                return;
            }
            drive(
                rx,
                requests,
                output,
                DesktopBackend {
                    demo,
                    attempt: 0,
                    #[cfg(target_os = "linux")]
                    client: connection::Client::new(connection::SessionBus),
                },
            )
            .await;
        },
    )
}

async fn drive(
    mut input: watch::Receiver<State>,
    mut requests: mpsc::Receiver<SystemRequest>,
    mut output: futures::channel::mpsc::Sender<Event>,
    mut backend: impl Backend,
) {
    let mut processed = 0;
    let mut system_closed = false;
    loop {
        tokio::select! {
            changed = input.changed() => if changed.is_err() { break; },
            request = requests.recv(), if !system_closed => {
                if let Some(request) = request {
                    if !request.receipt.is_closed() && request.cancellation.has_changed().is_ok() {
                        let result = backend.deliver(request.delivery).await.map_err(|error| format!("{error:#}"));
                        let _ = request.receipt.send(result);
                    }
                } else {
                    system_closed = true;
                }
                continue;
            }
        }
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
        let event = match backend.deliver(delivery.clone()).await {
            Ok(()) => Event::Sent(delivery),
            Err(error) => Event::Failed(delivery, format!("{error:#}")),
        };
        if output.send(event).await.is_err() {
            break;
        }
    }
}

fn saving_delivery() -> Delivery {
    Delivery {
        through: 0,
        count: 0,
        popups: true,
        sound: false,
        title: "Shep is finishing your changes".into(),
        body: "Shep will quit when your changes are saved. Open Shep from the tray to keep working, or choose Quit Shep to leave now; journaled uploads resume next time."
            .into(),
    }
}
