//! Cancellation interrupts read-only I/O; admitted journal/provider writes keep
//! their owner until acknowledgment. The coordinator owns the sending endpoint.
use std::future::Future;
use tokio::sync::watch;

#[derive(Clone, Default)]
pub struct Control {
    stop: Option<watch::Receiver<bool>>,
}
#[derive(Debug)]
pub struct Stopped;
impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Profile sync paused. Saved changes will be available next time.")
    }
}
impl std::error::Error for Stopped {}
impl Control {
    pub fn channel() -> (watch::Sender<bool>, Self) {
        let (send, stop) = watch::channel(false);
        (send, Self { stop: Some(stop) })
    }
    pub fn check(&self) -> anyhow::Result<()> {
        if self
            .stop
            .as_ref()
            .is_some_and(|s| *s.borrow() || s.has_changed().is_err())
        {
            return Err(Stopped.into());
        }
        Ok(())
    }
    pub async fn read<T>(
        &self,
        future: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        self.check()?;
        let Some(mut stop) = self.stop.clone() else {
            return future.await;
        };
        tokio::select! {biased;
            _=stop.wait_for(|stop|*stop) => Err(Stopped.into()),
            result=future => result,
        }
    }
}
