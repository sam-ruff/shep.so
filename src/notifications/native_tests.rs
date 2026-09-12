use super::*;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::mpsc;
use zbus::{Connection, connection::Builder, zvariant::OwnedValue};

struct Bus {
    child: std::process::Child,
    _directory: tempfile::TempDir,
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Debug)]
struct Request {
    name: String,
    replaces: u32,
    icon: String,
    title: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedValue>,
    timeout: i32,
}
struct Desktop {
    requests: mpsc::Sender<Request>,
    reject: Arc<AtomicBool>,
}
#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Desktop {
    #[allow(clippy::too_many_arguments)] // Defined by the native D-Bus protocol.
    async fn notify(
        &self,
        name: String,
        replaces: u32,
        icon: String,
        title: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        self.requests
            .send(Request {
                name,
                replaces,
                icon,
                title,
                body,
                actions,
                hints,
                timeout,
            })
            .await
            .unwrap();
        if self.reject.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::AccessDenied("fixture denied".into()));
        }
        Ok(42)
    }
}
async fn private_bus() -> (Bus, String, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let address = format!("unix:path={}/bus", directory.path().display());
    let child = std::process::Command::new("dbus-daemon")
        .args([
            "--session",
            "--nofork",
            "--nopidfile",
            &format!("--address={address}"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let bus = Bus {
        child,
        _directory: directory,
    };
    let connection = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(connection) = Builder::address(address.as_str()).unwrap().build().await {
                break connection;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (bus, address, connection)
}
fn delivery(sound: bool) -> Delivery {
    Delivery {
        through: 1,
        count: 1,
        popups: true,
        sound,
        title: "Fixture sender".into(),
        body: "<b>Private & subject</b>".into(),
    }
}
#[tokio::test]
async fn notification_bus_uses_own_identity_escaped_body_and_explicit_sound_policy() {
    let (_bus, address, client) = private_bus().await;
    let (tx, mut rx) = mpsc::channel(4);
    let reject = Arc::new(AtomicBool::new(false));
    let _service = Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            Desktop {
                requests: tx,
                reject: reject.clone(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    for sound in [true, false] {
        linux_popup(&client, &delivery(sound)).await.unwrap();
        let request = rx.recv().await.unwrap();
        assert_eq!(
            (
                request.name.as_str(),
                request.replaces,
                request.icon.as_str()
            ),
            ("Shep", 0, APP_ID)
        );
        assert_eq!(request.title, "Fixture sender");
        assert_eq!(request.body, "&lt;b&gt;Private &amp; subject&lt;/b&gt;");
        assert!(request.actions.is_empty());
        assert_eq!(request.timeout, -1);
        assert_eq!(
            <&str>::try_from(&request.hints["desktop-entry"]).unwrap(),
            APP_ID
        );
        assert_eq!(
            <&str>::try_from(&request.hints["category"]).unwrap(),
            "email.arrived"
        );
        assert_eq!(
            bool::try_from(&request.hints["suppress-sound"]).unwrap(),
            !sound
        );
        assert_eq!(request.hints.contains_key("sound-name"), sound);
        if sound {
            assert_eq!(
                <&str>::try_from(&request.hints["sound-name"]).unwrap(),
                "message-new-email"
            );
        }
    }
    reject.store(true, Ordering::SeqCst);
    let error = linux_popup(&client, &delivery(false)).await.unwrap_err();
    assert!(format!("{error:#}").contains("fixture denied"));
    rx.recv().await.unwrap();
    reject.store(false, Ordering::SeqCst);
    linux_popup(&client, &delivery(false)).await.unwrap();
}
#[tokio::test]
async fn notification_missing_desktop_and_sound_helper_failure_are_errors() {
    let (_bus, _address, client) = private_bus().await;
    assert!(linux_popup(&client, &delivery(false)).await.is_err());
    // Real subprocess lifecycle with harmless commands, never host audio.
    sound_helper("/usr/bin/true", &[]).await.unwrap();
    assert!(sound_helper("/usr/bin/false", &[]).await.is_err());
    assert!(
        sound_helper("/no-such-shep-fixture-helper", &[])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn notification_sender_stays_on_bus_between_arrivals_until_worker_is_dropped()
-> anyhow::Result<()> {
    use crate::notifications::connection::{Client, Transport};
    struct OwnedBus {
        address: String,
        names: mpsc::Sender<String>,
    }
    #[async_trait::async_trait]
    impl Transport for OwnedBus {
        type Connection = Connection;

        fn is_closed(&self, connection: Connection) -> bool {
            connection.is_closed()
        }

        async fn connect(&self) -> anyhow::Result<Connection> {
            let connection = Builder::address(self.address.as_str())?.build().await?;
            let name = connection
                .unique_name()
                .context("Expected a bus identity")?
                .to_string();
            self.names.send(name).await?;
            Ok(connection)
        }

        async fn popup(&self, connection: Connection, delivery: Delivery) -> anyhow::Result<()> {
            linux_popup(&connection, &delivery).await
        }

        async fn sound(&self) -> anyhow::Result<()> {
            anyhow::bail!("The popup lifetime test must not play sound")
        }
    }
    let (_bus, address, observer) = private_bus().await;
    let (requests, mut received) = mpsc::channel(4);
    let _service = Builder::address(address.as_str())?
        .name("org.freedesktop.Notifications")?
        .serve_at(
            "/org/freedesktop/Notifications",
            Desktop {
                requests,
                reject: Arc::new(AtomicBool::new(false)),
            },
        )?
        .build()
        .await?;
    let (names, mut connected) = mpsc::channel(4);
    let mut client = Client::new(OwnedBus { address, names });
    client.deliver(delivery(false)).await?;
    received
        .recv()
        .await
        .context("First notification missing")?;
    let name = connected.recv().await.context("Sender missing")?;
    let proxy = zbus::fdo::DBusProxy::new(&observer).await?;
    assert!(proxy.name_has_owner(name.as_str().try_into()?).await?);
    client.deliver(delivery(false)).await?;
    received
        .recv()
        .await
        .context("Second notification missing")?;
    assert!(
        connected.try_recv().is_err(),
        "Each arrival must reuse its sender"
    );
    assert!(proxy.name_has_owner(name.as_str().try_into()?).await?);
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), async {
        while proxy.name_has_owner(name.as_str().try_into()?).await? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}
