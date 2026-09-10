//! Capacity-one lifecycle signals. A receiver observes the latest intent even
//! when it starts waiting after the sender; no polling loop or shared lock map.
#[derive(Clone, Debug)]
pub struct Signal(tokio::sync::watch::Sender<bool>);

impl Default for Signal {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
}
impl Signal {
    pub fn get(&self) -> bool {
        *self.0.borrow()
    }
    pub fn set(&self, value: bool) {
        self.0.send_replace(value);
    }
    pub async fn requested(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|value| *value).await;
    }
}

/// Runs `exit` after `grace` unless the process has already ended. iced drops
/// its Tokio runtime after the event loop stops, and that drop joins every
/// running blocking task; a stalled transfer or credential call must not keep
/// a windowless process alive once every required acknowledgment is in.
pub fn bound_exit(grace: std::time::Duration, exit: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("shep-exit-deadline".into())
        .spawn(move || {
            std::thread::sleep(grace);
            exit();
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "Could not start the exit deadline");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stop_before_wait_and_stop_while_waiting_both_wake() {
        let signal = Signal::default();
        signal.set(true);
        signal.requested().await;
        signal.set(false);
        let waiter = signal.requested();
        tokio::pin!(waiter);
        assert!(futures::poll!(&mut waiter).is_pending());
        signal.set(true);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap();
    }

    #[test]
    fn exit_deadline_fires_only_after_its_grace_period() {
        let (sender, receiver) = std::sync::mpsc::channel();
        bound_exit(std::time::Duration::from_millis(200), move || {
            let _ = sender.send(());
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "normal shutdown must not be cut short"
        );
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the deadline must end a lingering shutdown");
    }
}
