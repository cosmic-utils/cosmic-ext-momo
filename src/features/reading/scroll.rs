//! Measure the conversation before deciding whether to reveal replies.
use cosmic::iced::{
    advanced::widget::{
        operation::{Outcome, Scrollable},
        Id, Operation,
    },
    Rectangle, Vector,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Geometry {
    viewport: Rectangle,
    content: Rectangle,
    caption: Option<Rectangle>,
    replies: Option<Rectangle>,
    offset: f32,
}
impl Geometry {
    pub fn target(self) -> Option<f32> {
        let caption = self.caption?;
        let replies = self.replies?;
        // Leave long captions in place for reading. Also avoid moving an already
        // visible reply section, including when the whole thread fits the window.
        if caption.height > 160.0_f32.min(self.viewport.height / 3.0) {
            return None;
        }
        let y = replies.y - self.content.y;
        if y - self.offset + 120.0 <= self.viewport.height {
            return None;
        }
        let target = (y - 12.0)
            .max(0.0)
            .min((self.content.height - self.viewport.height).max(0.0));
        (target > self.offset + 1.0).then_some(target)
    }
}

pub struct Measure {
    caption_id: Id,
    geometry: Geometry,
}
impl Measure {
    pub fn new(post: &str) -> Self {
        Self {
            caption_id: Id::new(format!("post-caption:{post}")),
            geometry: Geometry::default(),
        }
    }
}
impl Operation<Geometry> for Measure {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation<Geometry>)) {
        visit(self);
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.caption_id) {
            self.geometry.caption = Some(bounds);
        }
        if id == Some(&Id::new("thread-replies")) {
            self.geometry.replies = Some(bounds);
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new("main-scroll")) {
            self.geometry.viewport = bounds;
            self.geometry.content = content;
            self.geometry.offset = translation.y;
        }
    }
    fn finish(&self) -> Outcome<Geometry> {
        Outcome::Some(self.geometry)
    }
}

pub fn animated_offset(target: f32, elapsed: std::time::Duration) -> (f32, bool) {
    let progress = (elapsed.as_secs_f32() / 0.25).min(1.0);
    (target * (1.0 - (1.0 - progress).powi(3)), progress >= 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry(caption: f32, replies: f32) -> Geometry {
        Geometry {
            viewport: Rectangle {
                height: 500.,
                ..Rectangle::default()
            },
            content: Rectangle {
                height: 1500.,
                ..Rectangle::default()
            },
            caption: Some(Rectangle {
                height: caption,
                ..Rectangle::default()
            }),
            replies: Some(Rectangle {
                y: replies,
                ..Rectangle::default()
            }),
            offset: 0.,
        }
    }
    #[test]
    fn short_captions_reveal_offscreen_replies_but_long_and_visible_ones_stay_put() {
        assert_eq!(geometry(0., 700.).target(), Some(688.));
        assert_eq!(geometry(80., 700.).target(), Some(688.));
        assert_eq!(geometry(300., 700.).target(), None);
        assert_eq!(geometry(80., 300.).target(), None);
        let mut missing = geometry(80., 700.);
        missing.replies = None;
        assert_eq!(missing.target(), None);
    }
    #[test]
    fn animation_starts_at_top_and_finishes_at_target() {
        assert_eq!(
            animated_offset(600., std::time::Duration::ZERO),
            (0., false)
        );
        assert_eq!(
            animated_offset(600., std::time::Duration::from_millis(250)),
            (600., true)
        );
        assert_eq!(
            animated_offset(600., std::time::Duration::from_secs(1)),
            (600., true)
        );
    }
}
