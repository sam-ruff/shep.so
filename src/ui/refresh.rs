use super::{App, Tab};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Animation {
    started: Option<Instant>,
    angle: f32,
}

impl Animation {
    pub(super) fn start(&mut self, now: Instant) {
        // Queued/coalesced clicks retain the same continuous rotation.
        self.started.get_or_insert(now);
    }

    pub(super) fn stop(&mut self) {
        self.started = None;
        self.angle = 0.;
    }

    pub(super) fn advance(&mut self, now: Instant) {
        if let Some(started) = self.started {
            // One turn per second. Elapsed time avoids accumulating drift when
            // the window is hidden or presentation skips a frame.
            self.angle = now.saturating_duration_since(started).as_secs_f64().fract() as f32
                * std::f32::consts::TAU;
        }
    }

    pub(super) fn angle(&self) -> f32 {
        self.angle
    }
}

impl App {
    pub(super) fn refresh_animating(&self) -> bool {
        self.busy.contains("sync")
            && self.refresh.started.is_some()
            && self.tab == Tab::Mail
            && !self.full_reader
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Event, Message};
    use std::time::Duration;

    #[test]
    fn refresh_animation_keeps_phase_through_queued_clicks_and_rejects_late_frames() {
        let start = Instant::now();
        let mut animation = Animation::default();
        animation.start(start);
        animation.advance(start + Duration::from_millis(125));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_4).abs() < 0.001);
        animation.start(start + Duration::from_millis(200));
        animation.advance(start + Duration::from_millis(250));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_2).abs() < 0.001);
        animation.advance(start + Duration::from_millis(1_250));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_2).abs() < 0.001);
        animation.stop();
        animation.advance(start + Duration::from_millis(1_500));
        assert_eq!(animation.angle(), 0.);
    }

    #[test]
    fn refresh_animation_is_manual_only_and_stops_when_hidden_or_finished() {
        let (mut app, _) = App::new();
        let _ = app.handle(Message::Backend(Event::Busy(
            "background-sync".into(),
            true,
        )));
        assert!(!app.refresh_animating());
        let _ = app.handle(Message::RefreshFrame(Instant::now()));
        assert_eq!(app.refresh.angle(), 0.);
        let _ = app.handle(Message::Backend(Event::Busy("sync".into(), true)));
        assert!(app.refresh_animating());
        let _ = app.handle(Message::Backend(Event::Busy(
            "background-sync".into(),
            false,
        )));
        // A completed background cycle must not stop its queued manual refresh.
        assert!(app.refresh_animating());
        app.tab = Tab::Preferences;
        assert!(!app.refresh_animating());
        app.tab = Tab::Mail;
        app.full_reader = true;
        assert!(!app.refresh_animating());
        app.full_reader = false;
        assert!(app.refresh_animating());
        let _ = app.handle(Message::Backend(Event::MailSyncFinished(Err(
            "Try again".into()
        ))));
        // Only the scheduler knows whether a follow-up is still queued.
        assert!(app.refresh_animating());
        let _ = app.handle(Message::Backend(Event::Busy("sync".into(), false)));
        assert!(!app.refresh_animating());
        let _ = app.handle(Message::RefreshFrame(Instant::now()));
        assert_eq!(app.refresh.angle(), 0.);
        assert_eq!(app.notice.as_ref().unwrap().0, "Try again");
    }

    #[test]
    fn refresh_animation_does_not_start_when_the_workspace_rejects_the_request() {
        let (mut app, _) = App::new();
        let _ = app.handle(Message::Sync);
        assert!(!app.busy.contains("sync"));
        assert!(!app.refresh_animating());
        assert!(app.notice.is_some());
    }
}
