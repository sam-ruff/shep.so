//! Native tray integration owns no mail state. Desktop callbacks enqueue small
//! bounded actions; saving, shutdown and window ownership stay with the app.
use futures::{SinkExt, Stream};
use std::sync::Arc;
use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Quit,
}
#[derive(Debug, Clone)]
pub enum Event {
    Available(bool),
    Action(Action),
    SavingNotification(u64, Result<(), String>),
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    Initialize(Arc<Icon>, watch::Sender<Option<Action>>),
}
#[derive(Debug)]
pub struct Icon {
    #[cfg(not(target_os = "linux"))]
    rgba: Vec<u8>,
    #[cfg(not(target_os = "linux"))]
    size: u32,
    #[cfg(target_os = "linux")]
    sizes: Vec<(u32, Vec<u8>)>,
}
impl Icon {
    #[cfg(not(target_os = "linux"))]
    fn load() -> anyhow::Result<Self> {
        #[cfg(target_os = "macos")]
        let bytes = include_bytes!("../../assets/logo-symbolic.webp").as_slice();
        #[cfg(not(target_os = "macos"))]
        let bytes = include_bytes!("../../assets/tray-32.webp").as_slice();
        let image = image::load_from_memory(bytes)?.into_rgba8();
        Ok(Self {
            size: image.width(),
            rgba: image.into_raw(),
        })
    }

    #[cfg(target_os = "linux")]
    fn load() -> anyhow::Result<Self> {
        Ok(Self {
            sizes: [
                include_bytes!("../../assets/tray-16.webp").as_slice(),
                include_bytes!("../../assets/tray-18.webp").as_slice(),
                include_bytes!("../../assets/tray-20.webp").as_slice(),
                include_bytes!("../../assets/tray-22.webp").as_slice(),
                include_bytes!("../../assets/tray-24.webp").as_slice(),
                include_bytes!("../../assets/tray-32.webp").as_slice(),
                include_bytes!("../../assets/tray-36.webp").as_slice(),
                include_bytes!("../../assets/tray-40.webp").as_slice(),
                include_bytes!("../../assets/tray-44.webp").as_slice(),
                include_bytes!("../../assets/tray-48.webp").as_slice(),
                include_bytes!("../../assets/tray-64.webp").as_slice(),
            ]
            .into_iter()
            .map(|bytes| {
                let image = image::load_from_memory(bytes)?.into_rgba8();
                Ok((image.width(), image.into_raw()))
            })
            .collect::<anyhow::Result<_>>()?,
        })
    }
}

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(
        8,
        move |mut output: futures::channel::mpsc::Sender<Event>| async move {
            let permitted = !demo || fixture_permitted();
            if !permitted {
                let _ = output.send(Event::Available(false)).await;
                futures::future::pending::<()>().await;
                return;
            }
            let icon = match tokio::task::spawn_blocking(Icon::load).await {
                Ok(Ok(icon)) => Arc::new(icon),
                _ => {
                    let _ = output.send(Event::Available(false)).await;
                    return;
                }
            };
            let (actions, mut input) = watch::channel(None);
            #[cfg(target_os = "linux")]
            let (availability, mut available) = tokio::sync::watch::channel(false);
            #[cfg(target_os = "linux")]
            let service = linux::run(icon, actions.clone(), availability);
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            let service = {
                let actions = actions.clone();
                async move { actions.closed().await }
            };
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let service = {
                let actions = actions.clone();
                async move { actions.closed().await }
            };
            let forward = async move {
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                if output
                    .send(Event::Initialize(icon, actions.clone()))
                    .await
                    .is_err()
                {
                    return;
                }
                loop {
                    #[cfg(target_os = "linux")]
                    let event = tokio::select! {
                        biased;
                        changed = available.changed() => {
                            if changed.is_err() { break; }
                            Event::Available(*available.borrow_and_update())
                        }
                        changed = input.changed() => {
                            if changed.is_err() { break; }
                            let Some(action) = *input.borrow_and_update() else { continue; };
                            Event::Action(action)
                        }
                    };
                    #[cfg(not(target_os = "linux"))]
                    let event = {
                        if input.changed().await.is_err() {
                            break;
                        }
                        let Some(action) = *input.borrow_and_update() else {
                            continue;
                        };
                        Event::Action(action)
                    };
                    if output.send(event).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! { _ = service => {}, _ = forward => {} }
        },
    )
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn initialize(icon: Arc<Icon>, actions: watch::Sender<Option<Action>>) -> bool {
    native::initialize(icon, actions).is_ok()
}

#[cfg(all(feature = "test-support", target_os = "linux"))]
pub(crate) fn fixture_permitted() -> bool {
    let args: Vec<_> = std::env::args().collect();
    if !args.iter().any(|arg| arg == "--tray-fixture") {
        return false;
    }
    let Some(path) = args
        .windows(2)
        .find(|pair| pair[0] == "--test-state")
        .and_then(|pair| std::path::Path::new(&pair[1]).parent())
    else {
        return false;
    };
    let expected = format!("unix:path={}", path.join("tray-bus").display());
    std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .ok()
        .is_some_and(|address| {
            address == expected
                || address.strip_prefix(&expected).is_some_and(|tail| {
                    tail.strip_prefix(",guid=").is_some_and(|guid| {
                        guid.len() == 32 && guid.bytes().all(|c| c.is_ascii_hexdigit())
                    })
                })
        })
}
#[cfg(not(all(feature = "test-support", target_os = "linux")))]
pub(crate) fn fixture_permitted() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_icons_have_real_transparency_and_clean_outer_margins() {
        for bytes in [
            include_bytes!("../../assets/logo-light.webp").as_slice(),
            include_bytes!("../../assets/logo-dark.webp").as_slice(),
            include_bytes!("../../assets/launcher.png").as_slice(),
        ] {
            let image = image::load_from_memory(bytes).unwrap().into_rgba8();
            assert_eq!(image.dimensions(), (128, 128));
            let mut visible = 0;
            let mut opaque = 0;
            for (x, y, pixel) in image.enumerate_pixels() {
                if !(20..=112).contains(&x) || !(4..=124).contains(&y) {
                    assert_eq!(pixel[3], 0, "fringe at {x},{y}");
                }
                visible += usize::from(pixel[3] > 0);
                opaque += usize::from(pixel[3] == 255);
            }
            assert!((4500..6000).contains(&visible));
            assert!(opaque > 3500, "The dog's interior must remain opaque");
        }
        for bytes in [
            include_bytes!("../../assets/tray-32.webp").as_slice(),
            include_bytes!("../../assets/logo-symbolic.webp").as_slice(),
        ] {
            let image = image::load_from_memory(bytes).unwrap().into_rgba8();
            assert_eq!(image.get_pixel(0, 0)[3], 0);
            assert_eq!(image.get_pixel(image.width() - 1, 0)[3], 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tray_sizes_fill_the_slot_with_an_opaque_light_face() -> anyhow::Result<()> {
        let icon = Icon::load()?;
        assert_eq!(
            icon.sizes.iter().map(|(size, _)| *size).collect::<Vec<_>>(),
            [16, 18, 20, 22, 24, 32, 36, 40, 44, 48, 64]
        );
        for (size, rgba) in icon.sizes {
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            let image = image::RgbaImage::from_raw(size, size, rgba)
                .ok_or_else(|| anyhow::anyhow!("Invalid icon dimensions"))?;
            let rows: Vec<_> = image
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel[3] >= 128)
                .map(|(_, y, _)| y)
                .collect();
            let top = rows
                .iter()
                .min()
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Empty icon"))?;
            let bottom = rows
                .iter()
                .max()
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Empty icon"))?;
            assert!(bottom - top + 1 >= size * 9 / 10, "Padded {size}px icon");
            let face = image.get_pixel(size / 2, size / 2);
            assert!(
                face[0] > 220 && face[1] > 220 && face[2] > 210 && face[3] == 255,
                "Dark or translucent {size}px face: {face:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn native_action_signal_keeps_final_intent_when_receiver_is_busy() {
        let (actions, mut receiver) = watch::channel(None);
        // The former eight-event queue dropped both final gestures here.
        // An unread coalescing slot retains the newest intent without blocking.
        for final_action in [Action::Quit, Action::Open] {
            for _ in 0..32 {
                actions.send_replace(Some(Action::Open));
            }
            actions.send_replace(Some(final_action));
            receiver.changed().await.unwrap();
            assert_eq!(*receiver.borrow_and_update(), Some(final_action));
        }
    }
}
