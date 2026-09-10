use super::Delivery;
use anyhow::Context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Duration;

#[cfg(all(test, target_os = "linux"))]
#[path = "native_tests.rs"]
mod tests;

const APP_ID: &str = "so.shep.Shep";

#[cfg(target_os = "linux")]
pub(super) async fn deliver(delivery: Delivery) -> anyhow::Result<()> {
    if delivery.popups {
        let connection = tokio::time::timeout(Duration::from_secs(2), zbus::Connection::session())
            .await
            .context("Connecting to desktop notifications timed out")?
            .context(
                "Desktop notifications are unavailable. Check your desktop notification service",
            )?;
        linux_popup(&connection, &delivery).await?;
    } else if delivery.sound {
        // The desktop notification service owns popup sounds (and Do Not
        // Disturb). Sound-only mode uses the native sound-theme helper.
        sound_helper("canberra-gtk-play", &["--id=message-new-email", "--description=Shep new email"]).await
            .context("Could not play the mail sound. Install libcanberra-gtk3-bin and check your sound settings")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn linux_popup(connection: &zbus::Connection, delivery: &Delivery) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut hints = HashMap::from([
        ("desktop-entry", Value::from(APP_ID)),
        ("category", Value::from("email.arrived")),
        ("urgency", Value::from(1u8)),
        ("suppress-sound", Value::from(!delivery.sound)),
    ]);
    if delivery.sound {
        hints.insert("sound-name", Value::from("message-new-email"));
    }
    let body = escape(&delivery.body);
    let arguments = (
        "Shep",
        0u32,
        APP_ID,
        delivery.title.as_str(),
        body,
        Vec::<&str>::new(),
        hints,
        -1i32,
    );
    let request = connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &arguments,
    );
    let reply = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .context("The desktop did not acknowledge the notification")?
        .context(
            "Could not show the notification. Check desktop notification permissions for Shep",
        )?;
    let _: u32 = reply
        .body()
        .deserialize()
        .context("Invalid desktop notification acknowledgment")?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn sound_helper(program: &str, arguments: &[&str]) -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .context("Playing the notification sound timed out")??;
    anyhow::ensure!(
        status.success(),
        "The sound service rejected the mail sound"
    );
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) async fn deliver(delivery: Delivery) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        if delivery.popups {
            // Register this unpackaged application's own identity per user;
            // never borrow PowerShell's notification identity or require admin.
            let root = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
            let (key, _) =
                root.create_subkey(format!("Software\\Classes\\AppUserModelId\\{APP_ID}"))?;
            key.set_value("DisplayName", &"Shep")?;
            let mut notification = notify_rust::Notification::new();
            notification
                .app_id(APP_ID)
                .summary(&delivery.title)
                .body(&delivery.body);
            if delivery.sound {
                notification.sound_name("Mail");
            }
            notification.show().context(
                "Could not show the notification. Allow Shep notifications in Windows Settings",
            )?;
        } else if delivery.sound {
            #[link(name = "winmm")]
            unsafe extern "system" {
                fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
            }
            let alias: Vec<u16> = "MailBeep\0".encode_utf16().collect();
            // SND_ALIAS | SND_ASYNC: use the user's configured Windows mail sound.
            let accepted = unsafe { PlaySoundW(alias.as_ptr(), std::ptr::null_mut(), 0x10001) };
            anyhow::ensure!(
                accepted != 0,
                "Windows could not play the mail sound. Check your sound settings"
            );
        }
        Ok(())
    })
    .await
    .context("The Windows notification worker stopped")?
}

#[cfg(target_os = "macos")]
pub(super) async fn deliver(delivery: Delivery) -> anyhow::Result<()> {
    if delivery.popups {
        tokio::task::spawn_blocking(move || {
            // mac-notification-sys only allows setting the identity once. Keep
            // its first result; never fall through to another app's identity.
            static IDENTITY: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
            IDENTITY
                .get_or_init(|| {
                    notify_rust::set_application(APP_ID).map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(|error| anyhow::anyhow!("{error}"))
                .context("Install and open Shep.app before enabling macOS notifications")?;
            let mut notification = notify_rust::Notification::new();
            notification.summary(&delivery.title).body(&delivery.body);
            if delivery.sound {
                notification.sound_name("Glass");
            }
            notification.show().context(
                "Could not show the notification. Allow Shep notifications in System Settings",
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("The macOS notification worker stopped")??;
    } else if delivery.sound {
        sound_helper("/usr/bin/afplay", &["/System/Library/Sounds/Glass.aiff"])
            .await
            .context("Could not play the mail sound. Check your sound settings")?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(super) async fn deliver(_: Delivery) -> anyhow::Result<()> {
    anyhow::bail!("Native notifications are unavailable on this operating system")
}
