//! Native window ownership. Backend workers stay alive when the window is closed.
use super::*;
use crate::desktop_tray::{Action, Event};

/// Longest a windowless process may linger on abandoned background work.
#[cfg(not(test))]
const EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Default)]
pub(super) struct State {
    pub window: Option<iced::window::Id>,
    pub available: bool,
    pub hidden: bool,
    pub temporary: bool,
    pub exiting: bool,
    saving_fallback: bool,
    /// An explicit repeated Quit leaves journaled work for the next launch.
    pub insisted: bool,
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
        self.tray.hidden = false;
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
        #[cfg(target_os = "windows")]
        let badge = self.apply_desktop_overlay();
        #[cfg(not(target_os = "windows"))]
        let badge = Task::none();
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if let Some((icon, actions)) = self.tray.initialization.take() {
            return Task::batch([
                badge,
                iced::window::run(window, move |_| {
                    crate::desktop_tray::initialize(icon, actions)
                })
                .map(|available| Message::Tray(Event::Available(available))),
            ]);
        }
        Task::batch([badge, self.handle(Message::HtmlScaleRequest(window))])
    }

    #[cfg(target_os = "windows")]
    pub(super) fn apply_desktop_overlay(&self) -> Task<Message> {
        let (Some(window), Some(frame)) = (self.tray.window, &self.desktop_overlay) else {
            return Task::none();
        };
        // Old worker output cannot restore a badge just disabled by the user.
        if !self.tray.ready || frame.count != self.unread_badge_count() {
            return Task::none();
        }
        let frame = frame.clone();
        iced::window::run(window, move |native| {
            if let Err(error) = crate::desktop_badge::apply_overlay(native, frame) {
                tracing::debug!(%error, "Taskbar badge could not be updated");
            }
        })
        .discard()
    }

    fn hide_main_window(&mut self) -> Task<Message> {
        self.tray.hidden = true;
        self.tray.ready = false;
        self.tray
            .window
            .take()
            .map(iced::window::close)
            .unwrap_or_else(Task::none)
    }

    pub(super) fn request_main_close(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.pending_close.is_some() {
            // The visible fallback window told the user Shep closes here once
            // saved; closing it again is an explicit Quit. A repeated native
            // close event while hiding is not.
            return if self.tray.saving_fallback {
                self.quit_now(window)
            } else {
                Task::none()
            };
        }
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
            let previous_notice = self.notice.as_ref().map(|(_, _, at)| *at);
            if self.flush_pane_resize() {
                self.save_preferences();
            }
            self.flush_draft_saves(true);
            if self.new_error_since(previous_notice) {
                return Task::none();
            }
            return self.hide_main_window();
        }
        self.quit_main(window)
    }

    fn quit_main(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.pending_close.is_some() {
            return self.quit_now(window);
        }
        self.tray.saving_fallback = false;
        self.tray.insisted = false;
        self.handle(Message::WindowClose(window))
    }

    /// An explicit Quit leaves journaled work to the next launch. Anything
    /// else must still acknowledge, and the reason stays visible until it does.
    pub(super) fn quit_now(&mut self, window: iced::window::Id) -> Task<Message> {
        self.tray.insisted = true;
        let previous_notice = self.notice.as_ref().map(|(_, _, at)| *at);
        let task = self.handle(Message::WindowClose(window));
        if self.tray.exiting || self.pending_close.is_none() {
            return task;
        }
        let refreshed = self.notice.as_ref().map(|(_, _, at)| *at) != previous_notice;
        let reason = match &self.notice {
            Some((text, false, _)) if refreshed => text.trim_end_matches('…').to_owned(),
            _ => "Finishing the current mail change".to_owned(),
        };
        self.notice(
            format!("{reason}. Shep will quit as soon as this is saved."),
            false,
        );
        self.tray.temporary = false;
        self.tray.saving_fallback = true;
        if self.tray.window.is_none() {
            return Task::batch([task, self.open_main_window()]);
        }
        task
    }

    pub(super) fn continue_tray_close(&mut self) -> Task<Message> {
        let waiting = self.pending_close.is_some()
            && self.tray.available
            && self.composer.picker.is_none()
            && !self.tray.temporary
            && !self.tray.saving_fallback
            && (self.has_required_close_work() || self.bulk.jobs.iter().any(|job| job.running > 0));
        // Quit from an already hidden window announces the wait the same way.
        if !waiting || (self.tray.window.is_none() && !self.tray.hidden) {
            return Task::none();
        }
        self.tray.temporary = true;
        let notification = Task::perform(
            crate::desktop_tray::saving_notification(self.demo),
            |result| Message::Tray(Event::SavingNotification(result)),
        );
        Task::batch([self.hide_main_window(), notification])
    }

    pub(super) fn tray_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::SavingNotification(result) => {
                if result.is_err() && self.tray.temporary && !self.tray.exiting {
                    self.notice("Desktop notifications are unavailable. Shep will close here when your changes are saved; close again to leave now and resume journaled uploads next time.", false);
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
        self.tray.insisted = false;
        self.resume_folder_close_barrier();
        self.open_main_window()
    }

    pub(super) fn new_error_since(&self, previous: Option<Instant>) -> bool {
        self.notice
            .as_ref()
            .is_some_and(|(_, error, at)| *error && Some(*at) != previous)
    }

    pub(super) fn reopen_after_failed_close(
        &mut self,
        hidden_write_failed: bool,
        was_hidden_closing: bool,
    ) -> Task<Message> {
        if !self.tray.exiting
            && (hidden_write_failed
                || ((self.tray.temporary || was_hidden_closing)
                    && self.pending_close.is_none()
                    && self.composer.close.is_none()))
        {
            return self.restore_main_window();
        }
        Task::none()
    }

    pub(super) fn finish_exit(&mut self) -> Task<Message> {
        self.tray.exiting = true;
        self.tray.temporary = false;
        self.pending_close = None;
        // Every required acknowledgment is in; only abandoned background work
        // could still hold the runtime open after the last window is gone.
        #[cfg(not(test))]
        crate::lifecycle::bound_exit(EXIT_GRACE, || {
            tracing::warn!(
                "Background work did not stop within the exit grace period; exiting now"
            );
            std::process::exit(0);
        });
        iced::exit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_hidden_write_failure_reopens_but_old_errors_and_refreshes_do_not() {
        for durable_write in [false, true] {
            let (mut app, _) = App::new();
            let window = iced::window::Id::unique();
            app.tray.window = Some(window);
            app.tray.available = true;
            app.preferences.close_to_tray = true;
            app.busy
                .insert(if durable_write { "send:reply" } else { "sync" }.into());
            app.notice("An older error", true);
            let _ = app.update(Message::WindowCloseRequested(window));
            assert!(
                app.tray.window.is_none(),
                "An old error must not prevent hiding"
            );
            let _ = app.update(Message::Backend(crate::engine::Event::Busy(
                "sync".into(),
                false,
            )));
            assert!(
                app.tray.window.is_none(),
                "An old error must not reopen on progress"
            );
            let _ = app.update(Message::Backend(crate::engine::Event::Error(
                "New failure; retry".into(),
            )));
            assert_eq!(app.tray.window.is_some(), durable_write);
            assert!(!app.tray.exiting);
            assert!(app.pending_close.is_none());
            assert_eq!(app.notice.as_ref().unwrap().0, "New failure; retry");
        }
    }

    #[test]
    fn quit_after_ordinary_hide_reopens_on_failure_and_late_stop_cannot_exit() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.preferences.close_to_tray = true;
        app.bulk.stopped = true;
        app.busy.insert("send:reply".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.hidden);
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        assert!(app.pending_close.is_some());
        assert!(app.tray.window.is_none());
        let pending = app.pending_close;
        app.composer.io = Some("newer-attachment".into());
        let _ = app.update(Message::Backend(crate::engine::Event::DraftFiles(
            "older-attachment".into(),
            Err("Old failure".into()),
        )));
        assert_eq!(app.pending_close, pending);
        assert!(app.tray.window.is_none());
        let _ = app.update(Message::Backend(crate::engine::Event::Error(
            "Save failed; retry".into(),
        )));
        assert!(app.tray.window.is_some());
        assert!(!app.tray.hidden);
        assert!(app.pending_close.is_none());
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:reply".into(),
            false,
        )));
        assert!(!app.tray.exiting);
    }

    #[test]
    fn ordinary_hide_keeps_queue_rejected_draft_visible() {
        let (mut app, _) = App::new();
        let (sender, _receiver) = engine::CommandSender::persistence_test_channel();
        for _ in 0..32 {
            sender
                .try_send(Command::AutoSaveDraft(Draft::default()))
                .unwrap();
        }
        app.tx = Some(sender);
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.preferences.close_to_tray = true;
        app.load_draft(Draft {
            id: "unsaved".into(),
            ..Default::default()
        });
        app.edit_compose_field("subject", "Keep these words".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert_eq!(app.tray.window, Some(window));
        assert!(app.composer.current.dirty.is_some());
        assert!(app.composer.current.pending.is_none());
        assert!(app.notice.as_ref().unwrap().0.contains("queue is full"));
    }

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
    fn quit_from_temporary_tray_exits_when_only_journaled_work_remains() {
        for key in ["backup:one", "credential-cleanup"] {
            let (mut app, _) = App::new();
            let window = iced::window::Id::unique();
            app.tray.window = Some(window);
            app.tray.available = true;
            app.bulk.stopped = true;
            app.busy.insert(key.into());
            let _ = app.update(Message::WindowCloseRequested(window));
            assert!(app.tray.temporary, "{key}");
            assert_eq!(app.pending_close, Some(window), "{key}");
            assert!(!app.tray.exiting, "first close waits: {key}");
            let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
            assert!(app.tray.exiting, "Quit must leave now: {key}");
            assert!(app.tray.window.is_none(), "{key}");
        }
    }

    #[test]
    fn repeated_quit_with_unjournaled_save_reopens_with_reason_and_still_auto_exits() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.extend(["send:one".into(), "backup:one".into()]);
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.temporary);
        assert!(app.tray.window.is_none());
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        assert!(!app.tray.exiting, "an unjournaled send must still finish");
        assert!(app.tray.window.is_some(), "the reason must be visible");
        assert!(!app.tray.temporary);
        assert!(app.pending_close.is_some(), "Quit keeps close intent");
        let (text, error, _) = app.notice.as_ref().unwrap();
        assert!(text.contains("quit"), "{text}");
        assert!(!error);
        let _ = app.update(Message::Tick);
        assert!(app.tray.window.is_some(), "no repeated hiding");
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:one".into(),
            false,
        )));
        assert!(
            app.tray.exiting,
            "the journaled backup no longer holds exit"
        );
    }

    #[test]
    fn quit_from_ordinary_hidden_tray_notifies_and_second_quit_abandons_journaled_upload() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.preferences.close_to_tray = true;
        app.bulk.stopped = true;
        app.busy.insert("backup:one".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(app.tray.hidden);
        assert!(!app.tray.temporary);
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        assert!(app.pending_close.is_some());
        assert!(app.tray.temporary, "Quit while hidden must announce saving");
        assert!(!app.tray.exiting);
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        assert!(app.tray.exiting);
    }

    #[test]
    fn second_close_on_visible_saving_window_acts_as_quit_but_first_double_click_does_not() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.insert("backup:one".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        let _ = app.update(Message::Tray(Event::SavingNotification(Err(
            "No notification daemon".into(),
        ))));
        assert!(app.tray.window.is_some());
        assert_eq!(app.pending_close, Some(window));
        let visible = app.tray.window.unwrap();
        let _ = app.update(Message::WindowCloseRequested(visible));
        assert!(
            app.tray.exiting,
            "closing the visible saving window again leaves"
        );

        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.insert("backup:one".into());
        let _ = app.update(Message::WindowCloseRequested(window));
        let _ = app.update(Message::WindowCloseRequested(window));
        assert!(
            !app.tray.exiting,
            "a repeated native close event is not Quit"
        );
        assert!(app.tray.temporary);
    }

    #[test]
    fn restore_after_insisted_quit_cancels_abandonment() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.tray.window = Some(window);
        app.tray.available = true;
        app.bulk.stopped = true;
        app.busy.extend(["send:one".into(), "backup:one".into()]);
        let _ = app.update(Message::WindowCloseRequested(window));
        let _ = app.update(Message::Tray(Event::Action(Action::Quit)));
        let _ = app.update(Message::Tray(Event::Action(Action::Open)));
        assert!(app.pending_close.is_none());
        let visible = app.tray.window.unwrap();
        let _ = app.update(Message::Backend(crate::engine::Event::Busy(
            "send:one".into(),
            false,
        )));
        let _ = app.update(Message::WindowCloseRequested(visible));
        assert!(app.pending_close.is_some());
        assert!(
            !app.tray.exiting,
            "a fresh close waits for the backup again"
        );
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
