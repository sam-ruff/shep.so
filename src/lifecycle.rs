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
}
