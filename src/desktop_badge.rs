//! Native launcher integration. The UI only replaces a small watch value; IPC,
//! reconnects and desktop restarts stay on this independent async worker.
use futures::{SinkExt, Stream};
use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(target_os = "macos", test))]
mod count_delivery;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "windows", test))]
pub mod overlay;
#[cfg(target_os = "windows")]
mod windows;

pub const SUPPORTED: bool = cfg!(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos"
));

#[derive(Debug, Clone)]
pub enum Event {
    Ready(watch::Sender<u64>),
    #[cfg(any(target_os = "windows", test))]
    Overlay(std::sync::Arc<overlay::Frame>),
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
            #[cfg(target_os = "windows")]
            overlay::run(rx, output).await;
            #[cfg(target_os = "macos")]
            macos::run(rx).await;
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            {
                let mut rx = rx;
                while rx.changed().await.is_ok() {}
            }
        },
    )
}

#[cfg(target_os = "windows")]
pub fn apply_overlay(
    window: &dyn iced::window::Window,
    frame: std::sync::Arc<overlay::Frame>,
) -> anyhow::Result<()> {
    use iced::window::raw_window_handle::RawWindowHandle;
    let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
        anyhow::bail!("Taskbar overlay requires a Win32 window")
    };
    windows::apply(handle.hwnd.get(), frame)
}
