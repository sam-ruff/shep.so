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
            // A calm clockwise turn every 2.4 seconds. Positive rotation follows
            // screen coordinates (Y down); elapsed time avoids frame drift.
            self.angle = (now.saturating_duration_since(started).as_secs_f64() / 2.4).fract()
                as f32
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
        animation.advance(start + Duration::from_millis(300));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_4).abs() < 0.001);
        animation.start(start + Duration::from_millis(400));
        animation.advance(start + Duration::from_millis(600));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_2).abs() < 0.001);
        animation.advance(start + Duration::from_millis(3_000));
        assert!((animation.angle() - std::f32::consts::FRAC_PI_2).abs() < 0.001);
        animation.stop();
        animation.advance(start + Duration::from_millis(3_300));
        assert_eq!(animation.angle(), 0.);
    }

    #[test]
    fn refresh_animation_turns_clockwise_slowly_and_wraps_without_a_jump() {
        let start = Instant::now();
        let mut animation = Animation::default();
        animation.start(start);
        let mut previous = 0.;
        for milliseconds in [100, 600, 1_200, 1_800, 2_399] {
            animation.advance(start + Duration::from_millis(milliseconds));
            // SVG uses screen coordinates: increasing angles are clockwise.
            assert!(animation.angle() > previous);
            assert!(
                (animation.angle() - milliseconds as f32 / 2_400. * std::f32::consts::TAU).abs()
                    < 0.001
            );
            previous = animation.angle();
        }
        animation.advance(start + Duration::from_millis(2_400));
        assert_eq!(animation.angle(), 0.);
        animation.advance(start + Duration::from_millis(2_500));
        assert!((animation.angle() - std::f32::consts::TAU / 24.).abs() < 0.001);
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
