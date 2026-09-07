use iced::Rectangle;

pub(super) struct Geometry {
    pub track: Rectangle,
    pub thumb: Rectangle,
    max: f32,
}
impl Geometry {
    pub fn new(bounds: Rectangle, viewport: Rectangle, content: f32, pan: f32) -> Option<Self> {
        let clip = bounds.intersection(&viewport)?;
        if content <= bounds.width + 1. || clip.height < super::SCROLLBAR_SPACE {
            return None;
        }
        let track = Rectangle {
            x: clip.x,
            y: clip.y + clip.height - super::SCROLLBAR_SPACE,
            width: clip.width,
            height: super::SCROLLBAR_SPACE,
        };
        let max = content - bounds.width;
        let width = (clip.width * bounds.width / content).clamp(24_f32.min(clip.width), clip.width);
        Some(Self {
            track,
            thumb: Rectangle {
                x: track.x + pan.clamp(0., max) / max * (track.width - width),
                width,
                ..track
            },
            max,
        })
    }
    pub fn position(&self, pointer: f32, grab: f32) -> f32 {
        (pointer - self.track.x - grab).max(0.) / (self.track.width - self.thumb.width).max(1.)
            * self.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn horizontal_track_stays_in_the_visible_body_and_preserves_the_grab_offset() {
        let bounds = Rectangle {
            x: 600.,
            y: -200.,
            width: 400.,
            height: 2000.,
        };
        let viewport = Rectangle {
            y: 120.,
            height: 500.,
            ..bounds
        };
        let bar = Geometry::new(bounds, viewport, 1000., 300.).unwrap();
        assert_eq!(bar.track.y + bar.track.height, 620.);
        assert_eq!(bar.thumb.width, 160.);
        assert_eq!(bar.position(bar.thumb.x + 30., 30.), 300.);
        assert_eq!(bar.position(bar.track.x, 30.), 0.);
        assert!(Geometry::new(bounds, viewport, 400., 0.).is_none());
    }
}
