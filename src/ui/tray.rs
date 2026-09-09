//! Native window ownership. Backend workers stay alive when the window is closed.
use super::*;
use crate::desktop_tray::{Action, Event};

#[derive(Default)]
pub(super) struct State {
    pub window: Option<iced::window::Id>,
    pub available: bool,
    pub temporary: bool,
    pub exiting: bool,
    saving_fallback: bool,
    pub ready: bool,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    initialization: Option<(
        Arc<crate::desktop_tray::Icon>,
        tokio::sync::watch::Sender<Option<Action>>,
    )>,
}

pub(super) fn window_settings(size: Size) -> iced::window::Settings {
    iced::window::Settings {
        size,
        exit_on_close_request: false,
        min_size: Some(Size::new(900., 640.)),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "so.shep.Shep".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl App {
    pub(super) fn boot() -> (Self, Task<Message>) {
        let (mut app, initial) = Self::new();
        let opening = app.open_main_window();
        (app, Task::batch([initial, opening]))
    }

    fn open_main_window(&mut self) -> Task<Message> {
        if let Some(window) = self.tray.window {
            return iced::window::gain_focus(window);
        }
        let (window, open) = iced::window::open(window_settings(self.size));
        self.tray.window = Some(window);
        self.tray.ready = false;
        open.map(Message::MainWindowOpened)
    }

    pub(super) fn main_window_opened(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.tray.window != Some(window) {
            return Task::none();
        }
        self.tray.ready = true;
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if let Some((icon, actions)) = self.tray.initialization.take() {
            return iced::window::run(window, move |_| {
                crate::desktop_tray::initialize(icon, actions)
            })
            .map(|available| Message::Tray(Event::Available(available)));
        }
        self.handle(Message::HtmlScaleRequest(window))
    }

    fn hide_main_window(&mut self) -> Task<Message> {
        self.tray.ready = false;
        self.tray
            .window
            .take()
            .map(iced::window::close)
            .unwrap_or_else(Task::none)
    }

    pub(super) fn request_main_close(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.preferences.close_to_tray {
            if self.composer.picker.is_some() {
                self.notice(
                    "Finish choosing attachments before closing the window.",
                    false,
                );
                return Task::none();
            }
            if !self.tray.available {
                self.notice("The system tray is unavailable. Use Quit Shep in Preferences to close the app.", true);
                return Task::none();
            }
            if self.flush_pane_resize() {
                self.save_preferences();
            }
            self.flush_draft_saves(true);
            return self.hide_main_window();
        }
        self.quit_main(window)
    }

    fn quit_main(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.pending_close.is_none() {
            self.tray.saving_fallback = false;
        }
        self.handle(Message::WindowClose(window))
    }

    pub(super) fn continue_tray_close(&mut self) -> Task<Message> {
        if self.pending_close.is_some()
            && self.tray.available
            && self.tray.window.is_some()
            && self.composer.picker.is_none()
            && !self.tray.temporary
            && !self.tray.saving_fallback
            && (self.has_required_close_work() || self.bulk.jobs.iter().any(|job| job.running > 0))
        {
            self.tray.temporary = true;
            let notification = Task::perform(
                crate::desktop_tray::saving_notification(self.demo),
                |result| Message::Tray(Event::SavingNotification(result)),
            );
            return Task::batch([self.hide_main_window(), notification]);
        }
        Task::none()
    }

    pub(super) fn tray_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::SavingNotification(result) => {
                if result.is_err() && self.tray.temporary && !self.tray.exiting {
                    self.notice("Desktop notifications are unavailable. Shep will close here when your changes are saved.", false);
                    self.tray.temporary = false;
                    self.tray.saving_fallback = true;
                    return self.open_main_window();
                }
            }
            Event::Available(available) => {
                self.tray.available = available;
                if !available && self.tray.window.is_none() && !self.tray.exiting {
                    self.notice("The system tray is unavailable. Shep has reopened so you can finish your work.", true);
                    return self.restore_main_window();
                }
            }
            Event::Action(Action::Open) => return self.restore_main_window(),
            Event::Action(Action::Quit) => {
                let window = self.tray.window.unwrap_or_else(iced::window::Id::unique);
                return self.quit_main(window);
            }
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            Event::Initialize(icon, actions) => {
                self.tray.initialization = Some((icon, actions));
                if self.tray.ready
                    && let Some(window) = self.tray.window
                {
                    return self.main_window_opened(window);
                }
            }
        }
        Task::none()
    }

    fn restore_main_window(&mut self) -> Task<Message> {
        self.pending_close = None;
        self.composer.close = None;
        self.tray.temporary = false;
        self.resume_folder_close_barrier();
        self.open_main_window()
    }

    pub(super) fn reopen_after_failed_close(&mut self) -> Task<Message> {
        if self.tray.temporary
            && !self.tray.exiting
            && self.pending_close.is_none()
            && self.composer.close.is_none()
        {
            return self.restore_main_window();
        }
        Task::none()
    }

    pub(super) fn finish_exit(&mut self) -> Task<Message> {
        self.tray.exiting = true;
        self.tray.temporary = false;
        self.pending_close = None;
        iced::exit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_failure_keeps_saving_visible_without_repeated_hiding() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.insert("send:one".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.temporary);
        let _ = app.update(Message::Tray(Event::SavingNotification(Err(
            "No notification daemon".into(),
        ))));
        assert!(app.tray.window.is_some());
        assert!(!app.tray.temporary);
        assert_eq!(app.pending_close, Some(window));
        let _ = app.update(Message::Tick);
        assert!(app.tray.window.is_some());
        assert!(!app.tray.temporary);
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:one".into(),
            false,
        )));
        assert!(app.tray.exiting);
    }

    #[test]
    fn attachment_chooser_stays_visible_and_only_admitted_storage_hides() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.composer.io = Some("draft".into());
        app.composer.picker = Some("draft".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert_eq!(app.tray.window, Some(window));
        assert_eq!(app.pending_close, Some(window));
        app.composer.picker = None;
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "sync".into(),
            false,
        )));
        assert!(app.tray.temporary);
        assert!(app.tray.window.is_none());
        let _ = app.update(Message::Backend(crate::engine::Event::DraftFiles(
            "draft".into(),
            Err("Attachment save failed".into()),
        )));
        assert!(app.tray.window.is_some());
        assert!(app.pending_close.is_none());
    }

    #[test]
    fn close_to_tray_keeps_workers_running_and_restore_reuses_saved_size() {
        let (mut app, _) = App::new();
        let (sender, mut commands, _network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.preferences.close_to_tray = true;
        app.size = Size::new(1060., 720.);
        app.busy.insert("sync".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.window.is_none());
        assert!(!app.tray.exiting);
        assert!(!app.tray.temporary);
        assert!(app.pending_close.is_none());
        assert!(
            commands.try_recv().is_err(),
            "Ordinary tray hide must not stop jobs"
        );
        let _ = app.update(Message::Tray(Event::Action(Action::Open)));
        assert!(app.tray.window.is_some());
        assert_ne!(app.tray.window, Some(window));
        assert_eq!(app.size, Size::new(1060., 720.));
        assert!(app.busy.contains("sync"));
    }

    #[test]
    fn temporary_tray_drains_required_write_and_auto_quits_after_ack() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.extend(["send:one".into(), "sync".into()]);
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.temporary);
        assert_eq!(app.pending_close, Some(window));
        assert!(app.tray.window.is_none());
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:one".into(),
            false,
        )));
        assert!(app.tray.exiting);
        assert!(app.tray.window.is_none(), "Successful exit must not reopen");
    }

    #[test]
    fn failure_or_missing_host_reopens_temporary_window_and_keeps_error() {
        for host_lost in [false, true] {
            let (mut app, _) = App::new();
            let window = iced::window::Id::unique();
            app.tray.window = Some(window);
            app.tray.available = true;
            app.bulk.stopped = true;
            app.busy.insert("send:one".into());
            let _ = app.update(Message::WindowCloseRequested(window));
            let event = if host_lost {
                Message::Tray(Event::Available(false))
            } else {
                Message::Backend(crate::engine::Event::Error("Save failed; retry".into()))
            };
            let _ = app.update(event);
            assert!(app.tray.window.is_some());
            assert!(!app.tray.temporary);
            assert!(!app.tray.exiting);
            assert!(app.pending_close.is_none());
            assert!(app.notice.as_ref().unwrap().1);
            let _ = app.update(Message::Backend(crate::engine::Event::BulkStopped));
            assert!(!app.tray.exiting);
        }
    }

    #[test]
    fn absent_tray_keeps_enabled_app_accessible_but_explicit_quit_still_exits() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.preferences.close_to_tray = true;
        app.bulk.stopped = true;
        let _ = app.update(Message::WindowCloseRequested(window));
        assert_eq!(app.tray.window, Some(window));
        assert!(app.notice.as_ref().unwrap().0.contains("unavailable"));
        assert!(!app.tray.exiting);
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        assert!(app.tray.exiting);
    }

    #[test]
    fn opening_while_saving_cancels_exit_and_late_receipts_leave_window_open() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.insert("send:one".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        let _ = app.update(Message::Tray(Event::Action(Action::Open)));
        assert!(app.pending_close.is_none());
        assert!(app.composer.close.is_none());
        let restored = app.tray.window;
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:one".into(),
            false,
        )));
        let _ = app.update(Message::Backend(crate::engine::Event::BulkStopped));
        assert_eq!(app.tray.window, restored);
        assert!(!app.tray.exiting);
    }
}
