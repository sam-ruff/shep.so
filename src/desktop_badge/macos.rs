//! NSDockTile belongs to AppKit. Main-queue work is acknowledged before another
//! update is admitted, including when the iced window is hidden in the tray.
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;
use tokio::sync::{oneshot, watch};

pub(super) async fn run(counts: watch::Receiver<u64>) {
    super::count_delivery::run(counts, |count| async move {
        let (ack, done) = oneshot::channel();
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(main) = MainThreadMarker::new() {
                let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
                NSApplication::sharedApplication(main)
                    .dockTile()
                    .setBadgeLabel(label.as_deref());
            }
            let _ = ack.send(());
        });
        let _ = done.await;
    })
    .await;
}
