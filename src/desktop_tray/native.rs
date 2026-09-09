//! iced invokes initialization through window::run, on the running native event
//! thread. This is required by AppKit and Win32; never spawn a second GUI loop.
use super::*;
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
thread_local! {
    static TRAY: std::cell::RefCell<Option<TrayIcon>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn initialize(
    icon: Arc<Icon>,
    actions: watch::Sender<Option<Action>>,
) -> anyhow::Result<()> {
    TRAY.with(|owned| {
        if owned.borrow().is_some() {
            return Ok(());
        }
        let menu = Menu::new();
        let open = MenuItem::with_id("shep.open", "Open Shep", true, None);
        let quit = MenuItem::with_id("shep.quit", "Quit Shep", true, None);
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit])?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Shep")
            .with_icon(tray_icon::Icon::from_rgba(
                icon.rgba.clone(),
                icon.size,
                icon.size,
            )?)
            .with_menu(Box::new(menu))
            .build()?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                "shep.open" => Action::Open,
                "shep.quit" => Action::Quit,
                _ => return,
            };
            actions.send_replace(Some(action));
        }));
        *owned.borrow_mut() = Some(tray);
        Ok(())
    })
}
