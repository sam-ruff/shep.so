//! Native launcher integration. The UI only replaces a small watch value; IPC,
//! reconnects and desktop restarts stay on this independent async worker.
use futures::{SinkExt, Stream};
use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod linux;

pub const SUPPORTED: bool = cfg!(target_os = "linux");

#[derive(Debug, Clone)]
pub enum Event {
    Ready(watch::Sender<u64>),
}

pub fn subscription() -> impl Stream<Item = Event> {
    iced::stream::channel(
        1,
        |mut output: futures::channel::mpsc::Sender<Event>| async move {
            let (tx, rx) = watch::channel(0);
            if output.send(Event::Ready(tx)).await.is_err() {
                return;
            }
            #[cfg(target_os = "linux")]
            linux::run(rx).await;
            #[cfg(not(target_os = "linux"))]
            {
                let mut rx = rx;
                while rx.changed().await.is_ok() {}
            }
        },
    )
}
