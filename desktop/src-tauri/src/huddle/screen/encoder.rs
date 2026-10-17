//! Stateful H.264 encoding and bounded transport batching.

use base64::{engine::general_purpose::STANDARD, Engine};
use buzz_core_pkg::huddle_screen::{
    validate_access_unit, ScreenMessage, ScreenShare, VideoAccessUnit, MAX_BATCH_FRAMES,
    MAX_HEIGHT, MAX_VIDEO_BYTES, MAX_WIDTH, VIDEO_FPS,
};
use image::{DynamicImage, RgbaImage};
use openh264::{
    encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, Level, Profile,
        RateControlMode, UsageType,
    },
    formats::{RgbaSliceU8, YUVBuffer},
    OpenH264API,
};

const MAX_DOWNSCALE_STEPS: u8 = 4;

/// Stateful inter-frame encoder; never creates JPEG snapshots.
pub struct ScreenVideoEncoder {
    encoder: Encoder,
    dimensions: (u32, u32),
    max_dimensions: (u32, u32),
    downscale_steps: u8,
    codec: String,
    frames: Vec<VideoAccessUnit>,
    bytes: usize,
    sequence: u64,
    share: ScreenShare,
}

impl ScreenVideoEncoder {
    /// Construct a low-latency screen-content encoder with periodic IDRs.
    pub fn new(share: ScreenShare) -> Result<Self, String> {
        Self::configured(
            share,
            UsageType::ScreenContentRealTime,
            Level::Level_4_0,
            3_000_000,
            (MAX_WIDTH, MAX_HEIGHT),
        )
    }

    /// Use camera rate control while retaining the same bounded wire format.
    pub fn camera(share: ScreenShare) -> Result<Self, String> {
        Self::configured(
            share,
            UsageType::CameraVideoRealTime,
            Level::Level_3_1,
            1_200_000,
            (
                buzz_core_pkg::huddle_screen::MAX_CAMERA_WIDTH,
                buzz_core_pkg::huddle_screen::MAX_CAMERA_HEIGHT,
            ),
        )
    }

    fn configured(
        share: ScreenShare,
        usage: UsageType,
        level: Level,
        bitrate: u32,
        max_dimensions: (u32, u32),
    ) -> Result<Self, String> {
        let config = EncoderConfig::new()
            .usage_type(usage)
            .profile(Profile::Baseline)
            .level(level)
            .bitrate(BitRate::from_bps(bitrate))
            .rate_control_mode(RateControlMode::Bitrate)
            .max_frame_rate(FrameRate::from_hz(VIDEO_FPS as f32))
            .intra_frame_period(IntraFramePeriod::from_num_frames(VIDEO_FPS))
            .skip_frames(true);
        Ok(Self {
            encoder: Encoder::with_api_config(OpenH264API::from_source(), config)
                .map_err(|e| format!("Video encoder initialization failed: {e}"))?,
            dimensions: (0, 0),
            max_dimensions,
            downscale_steps: 0,
            codec: String::new(),
            frames: Vec::new(),
            bytes: 0,
            sequence: 0,
            share,
        })
    }

    /// Discard dependencies after congestion and start the next batch with an IDR.
    pub fn discontinuity(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.encoder.force_intra_frame();
    }

    /// Encode a captured RGBA frame at its actual monotonic capture timestamp.
    ///
    /// Emits zero or more bounded batches. Every frame's timestamp survives
    /// batching so the receiver can play 30 FPS rather than redraw per event.
    pub fn push(&mut self, image: RgbaImage, timestamp: u64) -> Result<Vec<ScreenMessage>, String> {
        let captured = image.dimensions();
        if image.width() < 2 || image.height() < 2 {
            return Err("The selected source has no usable video pixels".into());
        }
        let mut scaled =
            if image.width() > self.max_dimensions.0 || image.height() > self.max_dimensions.1 {
                DynamicImage::ImageRgba8(image)
                    .resize(
                        self.max_dimensions.0,
                        self.max_dimensions.1,
                        image::imageops::FilterType::Triangle,
                    )
                    .to_rgba8()
            } else {
                image
            };
        let mut emitted = Vec::new();
        for _ in 0..=MAX_DOWNSCALE_STEPS {
            let width = scaled.width() & !1;
            let height = scaled.height() & !1;
            if width < 16 || height < 16 {
                return Err(format!(
                    "The selected source is too small for video ({width}x{height}); resize it to at least 16x16"
                ));
            }
            if (width, height) != scaled.dimensions() {
                scaled = image::imageops::crop_imm(&scaled, 0, 0, width, height).to_image();
            }
            if self.dimensions != (width, height) {
                if let Some(batch) = self.flush() {
                    emitted.push(batch);
                }
                self.dimensions = (width, height);
                self.encoder.force_intra_frame();
            }
            let source = RgbaSliceU8::new(scaled.as_raw(), (width as usize, height as usize));
            let yuv = YUVBuffer::from_rgba8_source(source);
            let encoded = self
                .encoder
                .encode_at(&yuv, openh264::Timestamp::from_millis(timestamp / 1000))
                .map_err(|e| {
                    format!(
                        "Video encoding failed (captured {}x{}, encoded {width}x{height}): {e}",
                        captured.0, captured.1
                    )
                })?;
            if encoded.frame_type() == FrameType::Skip {
                return Ok(emitted);
            }
            let key = encoded.frame_type() == FrameType::IDR;
            let data = encoded.to_vec();
            if data.len() > MAX_VIDEO_BYTES {
                if self.downscale_steps >= MAX_DOWNSCALE_STEPS {
                    return Err(format!(
                        "Video frame exceeds the relay's media limit after resolution adjustment ({width}x{height}, {} bytes)",
                        data.len()
                    ));
                }
                // Bitrate control bounds the average, not an individual IDR.
                // A smaller frame restarts dependencies and fits the wire limit.
                self.max_dimensions = (width / 2, height / 2);
                self.downscale_steps += 1;
                scaled = DynamicImage::ImageRgba8(scaled)
                    .resize(
                        self.max_dimensions.0,
                        self.max_dimensions.1,
                        image::imageops::FilterType::Triangle,
                    )
                    .to_rgba8();
                continue;
            }
            if let Some(codec) = validate_access_unit(&data, key, width, height)? {
                self.codec = codec;
            }
            if self.bytes + data.len() > MAX_VIDEO_BYTES {
                if let Some(batch) = self.flush() {
                    emitted.push(batch);
                }
            }
            self.bytes += data.len();
            self.frames.push(VideoAccessUnit {
                timestamp,
                key,
                data: STANDARD.encode(data),
            });
            if self.frames.len() >= MAX_BATCH_FRAMES {
                if let Some(batch) = self.flush() {
                    emitted.push(batch);
                }
            }
            return Ok(emitted);
        }
        Err("Video resolution adjustment exhausted its retry limit".into())
    }

    /// Flush a transport batch without changing the video capture cadence.
    pub fn flush(&mut self) -> Option<ScreenMessage> {
        if self.frames.is_empty() {
            return None;
        }
        self.sequence += 1;
        self.bytes = 0;
        Some(ScreenMessage::Video {
            share: self.share.clone(),
            sequence: self.sequence,
            codec: self.codec.clone(),
            width: self.dimensions.0,
            height: self.dimensions.1,
            frames: std::mem::take(&mut self.frames),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn encoder() -> ScreenVideoEncoder {
        ScreenVideoEncoder::new(ScreenShare {
            session_id: uuid::Uuid::new_v4(),
            started_at: nostr::Timestamp::now().as_secs() * 1000,
            label: "Encoder regression".into(),
        })
        .unwrap()
    }

    #[test]
    fn tiny_sources_are_rejected_before_the_native_encoder_is_called() {
        let mut encoder = encoder();
        for (width, height) in [(2, 2), (8, 8), (14, 100), (100, 14), (3840, 16)] {
            let image = RgbaImage::from_pixel(width, height, image::Rgba([20, 70, 90, 255]));
            let error = encoder.push(image, 0).unwrap_err();
            assert!(
                error.contains("resize it to at least 16x16"),
                "{width}x{height}: {error}"
            );
            assert!(!error.contains("Native:5"), "{error}");
        }
        assert!(encoder.flush().is_none());
        encoder.push(RgbaImage::new(320, 180), 33_333).unwrap();
        assert!(encoder.flush().is_some());
    }

    #[test]
    fn desktop_sizes_and_window_resizes_keep_batches_and_idrs_consistent() {
        let mut encoder = encoder();
        let mut batches = Vec::new();
        for (index, (width, height, encoded_size)) in [
            (3440, 1440, (1920, 804)),
            (900, 651, (900, 650)),
            (3840, 2160, (1920, 1080)),
            (320, 180, (320, 180)),
            (16, 16, (16, 16)),
        ]
        .into_iter()
        .enumerate()
        {
            let image = RgbaImage::from_pixel(width, height, image::Rgba([20, 70, 90, 255]));
            batches.extend(encoder.push(image, index as u64 * 33_333).unwrap());
            assert_eq!(encoder.dimensions, encoded_size);
        }
        batches.push(encoder.flush().unwrap());
        assert_eq!(batches.len(), 5);
        for batch in batches {
            let ScreenMessage::Video {
                width,
                height,
                frames,
                ..
            } = batch
            else {
                panic!("expected video");
            };
            assert_eq!(frames.len(), 1);
            assert!(frames[0].key, "resizes must start with a new IDR");
            validate_access_unit(
                &STANDARD.decode(&frames[0].data).unwrap(),
                true,
                width,
                height,
            )
            .unwrap();
        }
    }

    #[test]
    fn complex_scene_after_quiet_video_adapts_without_ending_or_losing_pending_frames() {
        let mut encoder = encoder();
        let mut batches = Vec::new();
        for frame in 0..30 {
            batches.extend(
                encoder
                    .push(RgbaImage::new(1920, 804), frame * 1_000_000)
                    .unwrap(),
            );
        }
        let mut state = 1_u32;
        let complex = RgbaImage::from_fn(1920, 804, |_, _| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgba([
                (state >> 24) as u8,
                (state >> 16) as u8,
                (state >> 8) as u8,
                255,
            ])
        });
        batches.extend(encoder.push(complex.clone(), 30_000_000).unwrap());
        assert!(
            encoder.downscale_steps > 0,
            "the oversized frame must adapt"
        );
        assert!(encoder.downscale_steps <= MAX_DOWNSCALE_STEPS);
        let adapted_limit = encoder.max_dimensions;
        for frame in 1..=8 {
            batches.extend(
                encoder
                    .push(complex.clone(), 30_000_000 + frame * 33_333)
                    .unwrap(),
            );
            assert_eq!(encoder.max_dimensions, adapted_limit);
        }
        batches.extend(encoder.flush());
        let mut decoder = openh264::decoder::Decoder::new().unwrap();
        let mut old_frames = 0;
        let mut adapted_frames = 0;
        for batch in batches {
            let event = EventBuilder::new(
                Kind::Custom(buzz_core_pkg::kind::KIND_HUDDLE_SCREEN as u16),
                serde_json::to_string(&batch).unwrap(),
            )
            .tags([Tag::parse(["h", &uuid::Uuid::new_v4().to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
            buzz_core_pkg::huddle_screen::validate_screen_event(
                &event,
                nostr::Timestamp::now().as_secs(),
            )
            .unwrap();
            let ScreenMessage::Video {
                width,
                height,
                frames,
                ..
            } = batch
            else {
                panic!("expected video");
            };
            if width < 1920 {
                assert!(frames[0].key || adapted_frames > 0);
            }
            for frame in frames {
                let data = STANDARD.decode(&frame.data).unwrap();
                assert!(data.len() <= MAX_VIDEO_BYTES);
                validate_access_unit(&data, frame.key, width, height).unwrap();
                assert!(decoder.decode(&data).unwrap().is_some());
                if width == 1920 {
                    old_frames += 1;
                } else {
                    adapted_frames += 1;
                }
            }
        }
        assert_eq!(old_frames, 30, "pending full-size frames must not be lost");
        assert!(adapted_frames > 1, "sharing must continue after adjustment");
    }

    #[test]
    fn moving_video_is_inter_frame_encoded_and_batched_without_reducing_fps() {
        let share = ScreenShare {
            session_id: uuid::Uuid::new_v4(),
            started_at: nostr::Timestamp::now().as_secs() * 1000,
            label: "Moving test".into(),
        };
        let mut encoder = ScreenVideoEncoder::new(share).unwrap();
        let mut batches = Vec::new();
        for i in 0..60_u64 {
            let image = RgbaImage::from_fn(320, 180, |x, y| {
                let active = x.abs_diff((i as u32 * 5) % 320) < 20 && y < 100;
                image::Rgba(if active {
                    [240, 40, 80, 255]
                } else {
                    [20, 70, 90, 255]
                })
            });
            batches.extend(
                encoder
                    .push(image, i * 1_000_000 / u64::from(VIDEO_FPS))
                    .unwrap(),
            );
        }
        if let Some(batch) = encoder.flush() {
            batches.push(batch);
        }
        assert!(
            batches.len() < 12,
            "transport events must not be individual video frames"
        );
        let mut count = 0;
        let mut keys = 0;
        for batch in batches {
            let event = EventBuilder::new(
                Kind::Custom(buzz_core_pkg::kind::KIND_HUDDLE_SCREEN as u16),
                serde_json::to_string(&batch).unwrap(),
            )
            .tags([Tag::parse(["h", &uuid::Uuid::new_v4().to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
            buzz_core_pkg::huddle_screen::validate_screen_event(
                &event,
                nostr::Timestamp::now().as_secs(),
            )
            .unwrap();
            if let ScreenMessage::Video { frames, .. } = batch {
                count += frames.len();
                keys += frames.iter().filter(|frame| frame.key).count();
            }
        }
        assert_eq!(count, 60);
        assert!(keys >= 2);
        assert!(
            keys < count / 2,
            "delta frames are required, not a still-image slideshow"
        );
    }

    #[test]
    fn camera_mode_produces_bounded_inter_frame_video_with_periodic_keyframes() {
        let share = ScreenShare {
            session_id: uuid::Uuid::new_v4(),
            started_at: nostr::Timestamp::now().as_secs() * 1000,
            label: "Camera test".into(),
        };
        let mut encoder = ScreenVideoEncoder::camera(share).unwrap();
        let mut packets = Vec::new();
        for frame in 0..60 {
            let image = RgbaImage::from_fn(320, 180, |x, y| {
                image::Rgba([(x.wrapping_add(frame * 4) % 256) as u8, y as u8, 100, 255])
            });
            packets.extend(encoder.push(image, u64::from(frame) * 33_333).unwrap());
        }
        packets.extend(encoder.flush());
        let mut frames = 0;
        let mut keys = 0;
        let mut decoder = openh264::decoder::Decoder::new().unwrap();
        for packet in packets {
            let event = EventBuilder::new(
                Kind::Custom(buzz_core_pkg::kind::KIND_HUDDLE_CAMERA as u16),
                serde_json::to_string(&packet).unwrap(),
            )
            .tags([Tag::parse(["h", &uuid::Uuid::new_v4().to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
            buzz_core_pkg::huddle_screen::validate_camera_event(
                &event,
                nostr::Timestamp::now().as_secs(),
            )
            .unwrap();
            let ScreenMessage::Video {
                frames: batch,
                width,
                height,
                ..
            } = packet
            else {
                panic!("expected video");
            };
            assert!(width <= 1280 && height <= 720);
            for frame in batch {
                frames += 1;
                keys += usize::from(frame.key);
                assert!(decoder
                    .decode(&STANDARD.decode(frame.data).unwrap())
                    .unwrap()
                    .is_some());
            }
        }
        assert_eq!(frames, 60);
        assert!((2..30).contains(&keys));
    }
}
