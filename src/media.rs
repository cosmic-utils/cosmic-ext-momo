//! Attachment metadata and an owned, bounded GStreamer video player.
use cosmic::iced::core::image::Id;
use cosmic::iced::core::image::{Allocation, Handle};
use gstreamer as gst;
use gstreamer_video::prelude::*;

#[derive(Debug, Clone)]
pub struct Attachment {
    pub preview: Option<String>,
    pub url: String,
    pub description: Option<String>,
    pub video: bool,
    pub looping: bool,
}

impl Attachment {
    pub fn cached_image<'a>(
        &self,
        handles: &'a std::collections::HashMap<String, Handle>,
    ) -> Option<&'a Handle> {
        if !self.video {
            if let Some(original) = handles.get(&self.url) {
                return Some(original);
            }
        }
        self.preview.as_ref().and_then(|url| handles.get(url))
    }

    pub fn from_media(media: &megalodon::entities::Attachment) -> Option<Self> {
        use megalodon::entities::attachment::AttachmentType;
        let (video, looping) = match media.r#type {
            AttachmentType::Image => (false, false),
            AttachmentType::Video => (true, false),
            AttachmentType::Gifv => (true, true),
            _ => return None,
        };
        Some(Self {
            preview: media.preview_url.clone(),
            url: media.url.clone(),
            description: media.description.clone(),
            video,
            looping,
        })
    }
}

pub struct Player {
    pipeline: gst::Element,
    sink: gstreamer_app::AppSink,
    // Keep the allocation alive so the renderer cannot evict the displayed frame.
    frame: Option<Allocation>,
    pending_frame: Option<Id>,
    pub paused: bool,
    pub muted: bool,
    pub ended: bool,
    pub error: bool,
    pub position: f64,
    scrub_position: Option<f64>,
    pub duration: f64,
    looping: bool,
}

impl Player {
    pub fn new(url: &str, looping: bool) -> Result<Self, ()> {
        gst::init().map_err(|_| ())?;
        let sink = gstreamer_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .build(),
            )
            .max_buffers(1)
            .drop(true)
            .build();
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", url)
            .property("video-sink", &sink)
            .property("mute", looping)
            .build()
            .map_err(|_| ())?;
        let mut player = Self {
            pipeline,
            sink,
            frame: None,
            pending_frame: None,
            paused: false,
            muted: looping,
            ended: false,
            error: false,
            position: 0.0,
            scrub_position: None,
            duration: 0.0,
            looping,
        };
        if player.pipeline.set_state(gst::State::Playing).is_err() {
            player.fail();
            return Err(());
        }
        Ok(player)
    }

    fn fail(&mut self) {
        self.error = true;
        self.paused = true;
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    pub fn frame(&self) -> Option<&Handle> {
        self.frame.as_ref().map(Allocation::handle)
    }

    /// Publish only uploaded frames, ignoring replies from a previously opened player.
    pub fn frame_uploaded(
        &mut self,
        id: Id,
        result: Result<Allocation, cosmic::iced::runtime::image::Error>,
    ) {
        if self.pending_frame != Some(id) || self.error {
            return;
        }
        self.pending_frame = None;
        match result {
            Ok(frame) => self.frame = Some(frame),
            Err(_) => self.fail(),
        }
    }

    /// Return at most one frame for upload, retaining the visible frame until completion.
    pub fn tick(&mut self) -> Option<Handle> {
        if self.error {
            return None;
        }
        if let Some(bus) = self.pipeline.bus() {
            while let Some(message) = bus.pop() {
                match message.view() {
                    gst::MessageView::Error(_) => {
                        self.fail();
                        return None;
                    }
                    gst::MessageView::Eos(_) => {
                        if self.looping {
                            self.seek(0.0);
                        } else {
                            self.ended = true;
                            self.paused = true;
                            let _ = self.pipeline.set_state(gst::State::Paused);
                        }
                    }
                    _ => {}
                }
            }
        }
        // AppSink retains only the latest decoded sample while an upload is pending.
        // This bounds memory and avoids queuing obsolete frames when rendering is slow.
        let next_frame = if self.pending_frame.is_none() {
            self.sink
                .try_pull_sample(gst::ClockTime::ZERO)
                .and_then(|sample| {
                    let info = gstreamer_video::VideoInfo::from_caps(sample.caps()?).ok()?;
                    let frame = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(
                        sample.buffer()?,
                        &info,
                    )
                    .ok()?;
                    let data = frame.plane_data(0).ok()?;
                    let stride = usize::try_from(frame.plane_stride()[0]).ok()?;
                    let width = info.width() as usize * 4;
                    let height = info.height() as usize;
                    let mut pixels = Vec::with_capacity(width.checked_mul(height)?);
                    for y in 0..height {
                        pixels.extend_from_slice(data.get(y * stride..y * stride + width)?);
                    }
                    Some(Handle::from_rgba(info.width(), info.height(), pixels))
                })
        } else {
            None
        };
        if let Some(frame) = &next_frame {
            self.pending_frame = Some(frame.id());
        }
        self.position = self
            .pipeline
            .query_position::<gst::ClockTime>()
            .map_or(self.position, |t| t.nseconds() as f64 / 1e9);
        self.duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(self.duration, |t| t.nseconds() as f64 / 1e9);
        next_frame
    }

    pub fn display_position(&self) -> f64 {
        self.scrub_position.unwrap_or(self.position)
    }

    pub fn scrub(&mut self, seconds: f64) {
        if !self.error && seconds.is_finite() {
            self.scrub_position = Some(seconds.clamp(0.0, self.duration));
        }
    }

    pub fn finish_scrub(&mut self) {
        if let Some(position) = self.scrub_position.take() {
            self.seek(position);
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.error {
            return;
        }
        if self.ended {
            self.seek(0.0);
        }
        let state = if self.paused {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if self.pipeline.set_state(state).is_err() {
            self.fail();
        } else {
            self.paused = !self.paused;
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.pipeline.set_property("mute", self.muted);
    }

    pub fn seek(&mut self, seconds: f64) {
        if self.error || !seconds.is_finite() {
            return;
        }
        if self
            .pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_nseconds((seconds.max(0.0) * 1e9) as u64),
            )
            .is_ok()
        {
            self.ended = false;
            self.position = seconds.max(0.0);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn gallery_prefers_the_selected_original_and_keeps_previews_while_loading() {
        let mut handles = std::collections::HashMap::new();
        let mut item = Attachment {
            url: "original".into(),
            preview: Some("preview".into()),
            description: None,
            video: false,
            looping: false,
        };
        let preview = Handle::from_rgba(1, 1, vec![0, 0, 0, 255]);
        let original = Handle::from_rgba(1, 1, vec![255, 255, 255, 255]);
        handles.insert("preview".into(), preview.clone());
        assert_eq!(item.cached_image(&handles).unwrap().id(), preview.id());
        handles.insert("original".into(), original.clone());
        assert_eq!(item.cached_image(&handles).unwrap().id(), original.id());
        item.preview = None;
        assert_eq!(item.cached_image(&handles).unwrap().id(), original.id());
        item.url = "next".into();
        assert!(item.cached_image(&handles).is_none());
    }

    #[test]
    fn gallery_includes_videos_without_previews_and_preserves_media_types() {
        for (kind, expected) in [
            ("image", Some((false, false))),
            ("video", Some((true, false))),
            ("gifv", Some((true, true))),
            ("audio", None),
            ("unknown", None),
        ] {
            let media = serde_json::from_value(serde_json::json!({
                "id": "media", "type": kind, "url": "https://example.org/media"
            }))
            .unwrap();
            let item = Attachment::from_media(&media);
            assert_eq!(
                item.as_ref().map(|item| (item.video, item.looping)),
                expected
            );
            if let Some(item) = item {
                assert!(item.preview.is_none());
            }
        }
    }

    // Exercises actual decoding and lifecycle without a display, audio device, or server.
    #[test]
    fn video_decodes_seeks_replays_and_stops_on_drop() {
        gst::init().unwrap();
        let path = std::env::temp_dir().join(format!("momo-video-{}.webm", uuid::Uuid::new_v4()));
        let encoder = gst::parse::launch(&format!(
            "videotestsrc num-buffers=10 ! video/x-raw,width=32,height=24,framerate=10/1 ! vp8enc ! webmmux ! filesink location={}", path.display()
        )).unwrap();
        encoder.set_state(gst::State::Playing).unwrap();
        let message = encoder
            .bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(10),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .unwrap();
        encoder.set_state(gst::State::Null).unwrap();
        assert!(matches!(message.view(), gst::MessageView::Eos(_)));
        let url = reqwest::Url::from_file_path(&path).unwrap();
        let mut player = Player::new(url.as_str(), false).unwrap();
        let mut decoded_frames = 0;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !player.ended && !player.error && Instant::now() < deadline {
            if let Some(frame) = player.tick() {
                assert_eq!(player.pending_frame, Some(frame.id()));
                assert!(player.frame().is_none());
                decoded_frames += 1;
                // This headless decoder test has no renderer to acknowledge uploads.
                player.pending_frame = None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!player.error);
        assert!(decoded_frames > 1);
        assert!(player.duration > 0.0);
        assert!(player.ended && player.paused);
        player.toggle_mute();
        assert!(player.muted);
        player.toggle_pause();
        assert!(!player.ended && !player.paused);
        player.toggle_pause();
        assert!(player.paused);
        player.scrub(0.2);
        let _ = player.tick();
        assert_eq!(player.display_position(), 0.2);
        player.scrub(0.5);
        let _ = player.tick();
        assert_eq!(player.display_position(), 0.5);
        player.finish_scrub();
        assert!(player.scrub_position.is_none());
        assert_eq!(player.display_position(), 0.5);
        let (result, _, _) = player.pipeline.state(gst::ClockTime::from_seconds(5));
        result.unwrap();
        let actual = player.pipeline.query_position::<gst::ClockTime>().unwrap();
        assert!(actual >= gst::ClockTime::from_mseconds(450));
        assert!(actual <= gst::ClockTime::from_mseconds(550));
        let pipeline = player.pipeline.clone();
        drop(player);
        assert_eq!(pipeline.current_state(), gst::State::Null);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn large_frames_wait_for_upload_and_stale_completions_are_ignored() {
        gst::init().unwrap();
        let pipeline = gst::parse::launch(
            "videotestsrc is-live=true ! video/x-raw,format=RGBA,width=1080,height=1920,framerate=30/1 ! appsink name=frames",
        ).unwrap().downcast::<gst::Pipeline>().unwrap();
        // Use the same bounded sink behavior as the player without a codec dependency.
        let sink = {
            let existing = pipeline.by_name("frames").unwrap();
            let existing = existing.downcast::<gstreamer_app::AppSink>().unwrap();
            existing.set_max_buffers(1);
            existing.set_drop(true);
            existing
        };
        let mut player = Player {
            pipeline: pipeline.upcast(),
            sink,
            frame: None,
            pending_frame: None,
            paused: false,
            muted: true,
            ended: false,
            error: false,
            position: 0.0,
            scrub_position: None,
            duration: 0.0,
            looping: false,
        };
        player.pipeline.set_state(gst::State::Playing).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let frame = loop {
            if let Some(frame) = player.tick() {
                break frame;
            }
            assert!(!player.error && Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        let Handle::Rgba {
            width,
            height,
            ref pixels,
            ..
        } = frame
        else {
            panic!("expected decoded pixels");
        };
        assert_eq!((width, height), (1080, 1920));
        assert!(pixels.len() > 2 * 1024 * 1024);
        std::thread::sleep(Duration::from_millis(100));
        assert!(player.tick().is_none(), "only one upload may be in flight");
        assert!(
            player.frame().is_none(),
            "unallocated frames must not be displayed"
        );
        let stale_id = Handle::from_rgba(1, 1, vec![0; 4]).id();
        player.frame_uploaded(
            stale_id,
            Err(cosmic::iced::runtime::image::Error::OutOfMemory),
        );
        assert!(!player.error);
        assert_eq!(player.pending_frame, Some(frame.id()));
        player.frame_uploaded(
            frame.id(),
            Err(cosmic::iced::runtime::image::Error::OutOfMemory),
        );
        assert!(player.error);
        assert_eq!(player.pipeline.current_state(), gst::State::Null);
    }

    #[test]
    fn unavailable_video_reports_error_and_stops_pipeline() {
        let mut player = match Player::new("file:///nonexistent/momo-missing-video.webm", false) {
            Ok(player) => player,
            Err(()) => return, // Synchronous failure is also surfaced by the viewer.
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !player.error && Instant::now() < deadline {
            player.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(player.error);
        assert_eq!(player.pipeline.current_state(), gst::State::Null);
    }
}
