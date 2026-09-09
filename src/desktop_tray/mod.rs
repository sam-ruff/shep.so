//! Native tray integration owns no mail state. Desktop callbacks enqueue small
//! bounded actions; saving, shutdown and window ownership stay with the app.
use futures::{SinkExt, Stream};
use std::sync::Arc;
use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Quit,
}
#[derive(Debug, Clone)]
pub enum Event {
    Available(bool),
    Action(Action),
    SavingNotification(Result<(), String>),
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    Initialize(Arc<Icon>, watch::Sender<Option<Action>>),
}
#[derive(Debug)]
pub struct Icon {
    rgba: Vec<u8>,
    size: u32,
}
impl Icon {
    fn load() -> anyhow::Result<Self> {
        let source = image::load_from_memory(include_bytes!("../../assets/logo-light.webp"))?;
        let resized = source.resize_exact(32, 32, image::imageops::FilterType::Lanczos3);
        Ok(Self {
            rgba: resized.into_rgba8().into_raw(),
            size: 32,
        })
    }
}

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(
        8,
        move |mut output: futures::channel::mpsc::Sender<Event>| async move {
            let permitted = !demo || fixture_permitted();
            if !permitted {
                let _ = output.send(Event::Available(false)).await;
                futures::future::pending::<()>().await;
                return;
            }
            let icon = match tokio::task::spawn_blocking(Icon::load).await {
                Ok(Ok(icon)) => Arc::new(icon),
                _ => {
                    let _ = output.send(Event::Available(false)).await;
                    return;
                }
            };
            let (actions, mut input) = watch::channel(None);
            #[cfg(target_os = "linux")]
            let (availability, mut available) = tokio::sync::watch::channel(false);
            #[cfg(target_os = "linux")]
            let service = linux::run(icon, actions.clone(), availability);
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            let service = {
                let actions = actions.clone();
                async move { actions.closed().await }
            };
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let service = {
                let actions = actions.clone();
                async move { actions.closed().await }
            };
            let forward = async move {
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                if output
                    .send(Event::Initialize(icon, actions.clone()))
                    .await
                    .is_err()
                {
                    return;
                }
                loop {
                    #[cfg(target_os = "linux")]
                    let event = tokio::select! {
                        biased;
                        changed = available.changed() => {
                            if changed.is_err() { break; }
                            Event::Available(*available.borrow_and_update())
                        }
                        changed = input.changed() => {
                            if changed.is_err() { break; }
                            let Some(action) = *input.borrow_and_update() else { continue; };
                            Event::Action(action)
                        }
                    };
                    #[cfg(not(target_os = "linux"))]
                    let event = {
                        if input.changed().await.is_err() {
                            break;
                        }
                        let Some(action) = *input.borrow_and_update() else {
                            continue;
                        };
                        Event::Action(action)
                    };
                    if output.send(event).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! { _ = service => {}, _ = forward => {} }
        },
    )
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn initialize(icon: Arc<Icon>, actions: watch::Sender<Option<Action>>) -> bool {
    native::initialize(icon, actions).is_ok()
}

#[cfg(all(feature = "test-support", target_os = "linux"))]
fn fixture_permitted() -> bool {
    let args: Vec<_> = std::env::args().collect();
    if !args.iter().any(|arg| arg == "--tray-fixture") {
        return false;
    }
    let Some(path) = args
        .windows(2)
        .find(|pair| pair[0] == "--test-state")
        .and_then(|pair| std::path::Path::new(&pair[1]).parent())
    else {
        return false;
    };
    let expected = format!("unix:path={}", path.join("tray-bus").display());
    std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .ok()
        .is_some_and(|address| {
            address == expected
                || address.strip_prefix(&expected).is_some_and(|tail| {
                    tail.strip_prefix(",guid=").is_some_and(|guid| {
                        guid.len() == 32 && guid.bytes().all(|c| c.is_ascii_hexdigit())
                    })
                })
        })
}
#[cfg(not(all(feature = "test-support", target_os = "linux")))]
fn fixture_permitted() -> bool {
    false
}

pub async fn saving_notification(demo: bool) -> Result<(), String> {
    if !demo || fixture_permitted() {
        crate::notifications::saving_notification()
            .await
            .map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_action_signal_keeps_final_intent_when_receiver_is_busy() {
        let (actions, mut receiver) = watch::channel(None);
        // The former eight-event queue dropped both final gestures here.
        // An unread coalescing slot retains the newest intent without blocking.
        for final_action in [Action::Quit, Action::Open] {
            for _ in 0..32 {
                actions.send_replace(Some(Action::Open));
            }
            actions.send_replace(Some(final_action));
            receiver.changed().await.unwrap();
            assert_eq!(*receiver.borrow_and_update(), Some(final_action));
        }
    }
}
