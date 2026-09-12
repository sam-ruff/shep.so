use super::{Delivery, native};
use anyhow::Context;
use std::time::Duration;

#[cfg_attr(test, mockall::automock(type Connection = usize;))]
#[async_trait::async_trait]
pub(super) trait Transport: Send + Sync {
    type Connection: Clone + Send + Sync;

    async fn connect(&self) -> anyhow::Result<Self::Connection>;
    fn is_closed(&self, connection: Self::Connection) -> bool;
    async fn popup(&self, connection: Self::Connection, delivery: Delivery) -> anyhow::Result<()>;
    async fn sound(&self) -> anyhow::Result<()>;
}

pub(super) struct SessionBus;

#[async_trait::async_trait]
impl Transport for SessionBus {
    type Connection = zbus::Connection;

    fn is_closed(&self, connection: Self::Connection) -> bool {
        connection.is_closed()
    }

    async fn connect(&self) -> anyhow::Result<Self::Connection> {
        tokio::time::timeout(Duration::from_secs(2), zbus::Connection::session())
            .await
            .context("Connecting to desktop notifications timed out")?
            .context(
                "Desktop notifications are unavailable. Check your desktop notification service",
            )
    }

    async fn popup(&self, connection: Self::Connection, delivery: Delivery) -> anyhow::Result<()> {
        native::linux_popup(&connection, &delivery).await
    }

    async fn sound(&self) -> anyhow::Result<()> {
        native::sound_helper("canberra-gtk-play", &["--id=message-new-email", "--description=Shep new email"])
            .await
            .context("Could not play the mail sound. Install libcanberra-gtk3-bin and check your sound settings")
    }
}

pub(super) struct Client<T: Transport> {
    transport: T,
    connection: Option<T::Connection>,
}

impl<T: Transport> Client<T> {
    pub(super) fn new(transport: T) -> Self {
        Self {
            transport,
            connection: None,
        }
    }

    pub(super) async fn deliver(&mut self, delivery: Delivery) -> anyhow::Result<()> {
        if !delivery.popups {
            return if delivery.sound {
                self.transport.sound().await
            } else {
                Ok(())
            };
        }
        if self
            .connection
            .as_ref()
            .is_some_and(|connection| self.transport.is_closed(connection.clone()))
        {
            self.connection = None;
        }
        let connection = match &self.connection {
            Some(connection) => connection.clone(),
            None => {
                let connection = self.transport.connect().await?;
                // GNOME removes an installed app's notifications when this sender disconnects.
                self.connection = Some(connection.clone());
                connection
            }
        };
        let result = self.transport.popup(connection.clone(), delivery).await;
        if result.is_err() && self.transport.is_closed(connection) {
            // Reconnect for the next arrival; an uncertain request must not be replayed.
            self.connection = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::{Sequence, predicate::eq};

    fn delivery(through: u64) -> Delivery {
        Delivery {
            through,
            count: 1,
            popups: true,
            sound: false,
            title: "Fictional sender".into(),
            body: "Fictional subject".into(),
        }
    }

    #[tokio::test]
    async fn notification_connection_is_reused_and_released_with_its_owner() -> anyhow::Result<()> {
        let mut transport = MockTransport::new();
        transport
            .expect_is_closed()
            .with(eq(41))
            .times(1)
            .returning(|_| false);
        transport.expect_connect().times(1).returning(|| Ok(41));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(1)))
            .times(1)
            .returning(|_, _| Ok(()));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(2)))
            .times(1)
            .returning(|_, _| Ok(()));
        let mut client = Client::new(transport);
        client.deliver(delivery(1)).await?;
        client.deliver(delivery(2)).await?;
        assert_eq!(client.connection, Some(41));
        drop(client);
        Ok(())
    }

    #[tokio::test]
    async fn notification_transport_failure_reconnects_only_for_a_new_arrival() -> anyhow::Result<()>
    {
        let mut transport = MockTransport::new();
        transport
            .expect_is_closed()
            .with(eq(41))
            .times(1)
            .returning(|_| true);
        let mut sequence = Sequence::new();
        transport
            .expect_connect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Ok(41));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(1)))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Err(anyhow::anyhow!("acknowledgment lost")));
        transport
            .expect_connect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Ok(42));
        transport
            .expect_popup()
            .with(eq(42), eq(delivery(2)))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Ok(()));
        let mut client = Client::new(transport);
        assert!(client.deliver(delivery(1)).await.is_err());
        assert!(client.connection.is_none());
        client.deliver(delivery(2)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn notification_connection_error_and_muted_modes_do_not_send_popups() -> anyhow::Result<()>
    {
        let mut transport = MockTransport::new();
        transport
            .expect_connect()
            .times(1)
            .returning(|| Err(anyhow::anyhow!("desktop unavailable")));
        transport.expect_popup().never();
        transport.expect_sound().times(1).returning(|| Ok(()));
        let mut client = Client::new(transport);
        assert!(client.deliver(delivery(1)).await.is_err());
        let mut muted = delivery(2);
        muted.popups = false;
        client.deliver(muted.clone()).await?;
        muted.sound = true;
        client.deliver(muted).await?;
        assert!(client.connection.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn notification_service_rejection_keeps_existing_notifications_alive()
    -> anyhow::Result<()> {
        let mut transport = MockTransport::new();
        transport.expect_connect().times(1).returning(|| Ok(41));
        transport
            .expect_is_closed()
            .with(eq(41))
            .times(3)
            .returning(|_| false);
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(1)))
            .times(1)
            .returning(|_, _| Ok(()));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(2)))
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("permission rejected")));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(3)))
            .times(1)
            .returning(|_, _| Ok(()));
        let mut client = Client::new(transport);
        client.deliver(delivery(1)).await?;
        assert!(client.deliver(delivery(2)).await.is_err());
        assert_eq!(client.connection, Some(41));
        client.deliver(delivery(3)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn notification_closed_bus_reconnects_before_submitting_the_next_arrival()
    -> anyhow::Result<()> {
        let mut transport = MockTransport::new();
        let mut sequence = Sequence::new();
        transport
            .expect_connect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Ok(41));
        transport
            .expect_popup()
            .with(eq(41), eq(delivery(1)))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Ok(()));
        transport
            .expect_is_closed()
            .with(eq(41))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| true);
        transport
            .expect_connect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Ok(42));
        transport
            .expect_popup()
            .with(eq(42), eq(delivery(2)))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Ok(()));
        let mut client = Client::new(transport);
        client.deliver(delivery(1)).await?;
        client.deliver(delivery(2)).await?;
        Ok(())
    }
}
