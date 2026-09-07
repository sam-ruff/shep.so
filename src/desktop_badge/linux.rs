use futures::StreamExt;
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;
use zbus::{Connection, connection::Builder, zvariant::OwnedValue};

const PATH: &str = "/so/shep/Shep/Launcher";
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const URI: &str = "application://so.shep.Shep.desktop";

fn properties(count: u64) -> HashMap<&'static str, OwnedValue> {
    HashMap::from([
        ("count", OwnedValue::from(count.min(i64::MAX as u64) as i64)),
        ("count-visible", OwnedValue::from(count > 0)),
    ])
}

struct Entry(watch::Receiver<u64>);
#[zbus::interface(name = "com.canonical.Unity.LauncherEntry")]
impl Entry {
    fn query(&self) -> (&'static str, HashMap<&'static str, OwnedValue>) {
        (URI, properties(*self.0.borrow()))
    }
}

async fn publish(connection: &Connection, count: u64) -> zbus::Result<()> {
    let body = (URI, properties(count));
    let send = connection.emit_signal(None::<&str>, PATH, INTERFACE, "Update", &body);
    tokio::time::timeout(Duration::from_secs(2), send)
        .await
        .map_err(|_| zbus::Error::Failure("Launcher update timed out".into()))?
}

async fn connected(connection: &Connection, rx: &mut watch::Receiver<u64>) -> zbus::Result<()> {
    let proxy = zbus::fdo::DBusProxy::new(connection).await?;
    // Subscribe before the initial publication, so a shell appearing during
    // startup cannot miss it. Dash to Dock uses this same well-known owner.
    let mut owners = proxy
        .receive_name_owner_changed_with_args(&[(0, "com.canonical.Unity")])
        .await?;
    let count = *rx.borrow_and_update();
    publish(connection, count).await?;
    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    publish(connection, 0).await?;
                    return Ok(());
                }
            }
            owner = owners.next() => {
                if owner.is_none() { return Ok(()); }
            }
            _ = connection.closed() => return Ok(()),
        }
        let count = *rx.borrow_and_update();
        publish(connection, count).await?;
    }
}

pub(super) async fn run(mut rx: watch::Receiver<u64>) {
    run_at(&mut rx, None, Duration::from_secs(5)).await;
}

async fn run_at(rx: &mut watch::Receiver<u64>, address: Option<&str>, retry: Duration) {
    loop {
        let connect = async {
            match address {
                Some(address) => Builder::address(address)?,
                None => Builder::session()?,
            }
            .serve_at(PATH, Entry(rx.clone()))?
            .build()
            .await
        };
        if let Ok(Ok(connection)) = tokio::time::timeout(Duration::from_secs(2), connect).await {
            let _ = connected(&connection, rx).await;
            let _ = tokio::time::timeout(Duration::from_secs(2), connection.close()).await;
        }
        if rx.has_changed().is_err() {
            return;
        }
        // A missing session bus is normal on some desktops. Keep the latest
        // value through failure without spinning or involving the mail worker.
        tokio::time::sleep(retry).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::{MatchRule, MessageStream, message::Type};

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
    async fn bus() -> (Bus, String, Connection) {
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
                if let Ok(c) = Builder::address(address.as_str()).unwrap().build().await {
                    return c;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        (bus, address, connection)
    }
    async fn next(stream: &mut MessageStream, expected: u64) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let message = stream.next().await.unwrap().unwrap();
                let (uri, values): (String, HashMap<String, OwnedValue>) =
                    message.body().deserialize().unwrap();
                assert_eq!(uri, URI);
                let count = i64::try_from(&values["count"]).unwrap();
                assert_eq!(bool::try_from(&values["count-visible"]).unwrap(), count > 0);
                if count == expected as i64 {
                    return;
                }
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn actual_bus_receives_updates_query_zero_and_shell_restart_republication() {
        let (_bus, address, observer) = bus().await;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(INTERFACE)
            .unwrap()
            .member("Update")
            .unwrap()
            .build();
        let mut messages = MessageStream::for_match_rule(rule, &observer, Some(16))
            .await
            .unwrap();
        let (tx, mut rx) = watch::channel(0);
        let service = Builder::address(address.as_str())
            .unwrap()
            .serve_at(PATH, Entry(rx.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let connection = service.clone();
        let worker = tokio::spawn(async move {
            connected(&connection, &mut rx).await.unwrap();
        });
        next(&mut messages, 0).await;
        tx.send_replace(2);
        next(&mut messages, 2).await;
        let proxy = zbus::Proxy::new(
            &observer,
            service.unique_name().unwrap().as_str(),
            PATH,
            INTERFACE,
        )
        .await
        .unwrap();
        let (uri, values): (String, HashMap<String, OwnedValue>) =
            proxy.call("Query", &()).await.unwrap();
        assert_eq!(uri, URI);
        assert_eq!(i64::try_from(&values["count"]).unwrap(), 2);
        observer.request_name("com.canonical.Unity").await.unwrap();
        next(&mut messages, 2).await;
        observer.release_name("com.canonical.Unity").await.unwrap();
        next(&mut messages, 2).await;
        observer.request_name("com.canonical.Unity").await.unwrap();
        next(&mut messages, 2).await;
        for count in 3..=500 {
            tx.send_replace(count);
        }
        next(&mut messages, 500).await;
        tx.send_replace(0);
        next(&mut messages, 0).await;
        drop(tx);
        next(&mut messages, 0).await;
        tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn reconnect_after_a_lost_bus_publishes_the_latest_replacement_count() {
        let (mut bus, address, observer) = bus().await;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(INTERFACE)
            .unwrap()
            .member("Update")
            .unwrap()
            .build();
        let mut messages = MessageStream::for_match_rule(rule.clone(), &observer, Some(16))
            .await
            .unwrap();
        let (tx, mut rx) = watch::channel(3);
        let endpoint = address.clone();
        let worker = tokio::spawn(async move {
            run_at(&mut rx, Some(&endpoint), Duration::from_millis(20)).await;
        });
        next(&mut messages, 3).await;
        bus.child.kill().unwrap();
        bus.child.wait().unwrap();
        tx.send_replace(8);
        tx.send_replace(12);
        let _ = std::fs::remove_file(bus._directory.path().join("bus"));
        bus.child = std::process::Command::new("dbus-daemon")
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
        let observer = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(connection) = Builder::address(address.as_str()).unwrap().build().await {
                    break connection;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut messages = MessageStream::for_match_rule(rule, &observer, Some(16))
            .await
            .unwrap();
        observer.request_name("com.canonical.Unity").await.unwrap();
        next(&mut messages, 12).await;
        drop(tx);
        next(&mut messages, 0).await;
        tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap();
    }
}
