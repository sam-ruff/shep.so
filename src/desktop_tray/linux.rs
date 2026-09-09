use super::*;
use ksni::TrayMethods;

struct Tray {
    icon: Arc<Icon>,
    actions: watch::Sender<Option<Action>>,
    availability: tokio::sync::watch::Sender<bool>,
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "so.shep.Shep".into()
    }
    fn title(&self) -> String {
        "Shep".into()
    }
    fn icon_name(&self) -> String {
        "so.shep.Shep-symbolic".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let mut argb = self.icon.rgba.clone();
        for pixel in argb.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: self.icon.size as i32,
            height: self.icon.size as i32,
            data: argb,
        }]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.actions.send_replace(Some(Action::Open));
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: "Open Shep".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.actions.send_replace(Some(Action::Open));
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: "Quit Shep".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.actions.send_replace(Some(Action::Quit));
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        self.availability.send_replace(true);
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.availability.send_replace(false);
        true
    }
}

pub(super) async fn run(
    icon: Arc<Icon>,
    actions: watch::Sender<Option<Action>>,
    availability: tokio::sync::watch::Sender<bool>,
) {
    loop {
        let tray = Tray {
            icon: icon.clone(),
            actions: actions.clone(),
            availability: availability.clone(),
        };
        match tray.spawn().await {
            Ok(handle) => {
                availability.send_replace(true);
                loop {
                    tokio::select! {
                        _ = actions.closed() => { handle.shutdown().await; return; }
                        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                            if handle.is_closed() { break; }
                        }
                    }
                }
                availability.send_replace(false);
            }
            Err(_) => {
                availability.send_replace(false);
                tokio::select! {
                    _ = actions.closed() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watcher_loss_is_retained_even_with_unobserved_native_actions() {
        let (actions, _input) = watch::channel(None);
        for _ in 0..8 {
            actions.send_replace(Some(Action::Open));
        }
        let (availability, current) = tokio::sync::watch::channel(true);
        let tray = Tray {
            icon: Arc::new(Icon::load().unwrap()),
            actions,
            availability,
        };
        <Tray as ksni::Tray>::watcher_offline(&tray, ksni::OfflineReason::No);
        assert!(!*current.borrow());
        <Tray as ksni::Tray>::watcher_online(&tray);
        assert!(*current.borrow());
    }
}
