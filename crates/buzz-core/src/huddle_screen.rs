//! Real-time H.264 screen video over authenticated, ephemeral relay events.
//! A transport batch contains several timed access units, not still images.

use std::io::Read;

use base64::{engine::general_purpose::STANDARD, Engine};
use h264_reader::{
    annexb::AnnexBReader,
    nal::{sps::SeqParameterSet, Nal, RefNal, UnitType},
    push::NalInterest,
};
use nostr::Event;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum compressed bytes in one batch, including its IDR frame.
pub const MAX_VIDEO_BYTES: usize = 176 * 1024;
/// Maximum JSON content size; the signed envelope fits a 256 KiB socket limit.
pub const MAX_CONTENT_BYTES: usize = 244 * 1024;
/// Maximum encoded width.
pub const MAX_WIDTH: u32 = 1920;
/// Maximum encoded height.
pub const MAX_HEIGHT: u32 = 1080;
/// Target capture and playback cadence, independent of relay event rate.
pub const VIDEO_FPS: u32 = 30;
/// Maximum access units per transport event.
pub const MAX_BATCH_FRAMES: usize = 8;
/// Freshness window for live-only media.
pub const MAX_EVENT_AGE_SECS: u64 = 5;

/// Explicit share identity, retained across audio reconnects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScreenShare {
    /// Random UUID allocated at user consent.
    pub session_id: Uuid,
    /// Unix milliseconds when consent was granted.
    pub started_at: u64,
    /// Local display/window label.
    pub label: String,
}

/// One encoded Annex B H.264 access unit with its capture timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoAccessUnit {
    /// Microseconds since the beginning of this share.
    pub timestamp: u64,
    /// True only for an IDR with in-band SPS/PPS.
    pub key: bool,
    /// Standard base64 Annex B data, not an image or URL.
    pub data: String,
}

/// Wire message for a live H.264 stream or its explicit termination.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenMessage {
    /// A small batch of timed video frames; transport cadence is not video FPS.
    Video {
        /// Consent identity.
        share: ScreenShare,
        /// Monotonic batch number, used to detect discontinuities.
        sequence: u64,
        /// RFC 6381 AVC codec string derived from the stream SPS.
        codec: String,
        /// Encoded width, verified against every in-band SPS.
        width: u32,
        /// Encoded height, verified against every in-band SPS.
        height: u32,
        /// Timed H.264 access units in capture order.
        frames: Vec<VideoAccessUnit>,
    },
    /// Terminal notification; disconnected streams also expire on the client.
    Stopped {
        /// Consent identity.
        share: ScreenShare,
    },
}

/// Inspect an access unit without decoding pixels.
///
/// SPS dimensions and codec bytes come from the actual compressed stream,
/// preventing a sender from declaring small dimensions for an oversized video.
/// Returns the codec string when an SPS is present.
pub fn validate_access_unit(
    bytes: &[u8],
    key: bool,
    width: u32,
    height: u32,
) -> Result<Option<String>, String> {
    if bytes.is_empty()
        || bytes.len() > MAX_VIDEO_BYTES
        || !(bytes.starts_with(&[0, 0, 1]) || bytes.starts_with(&[0, 0, 0, 1]))
    {
        return Err("invalid: malformed H.264 access unit".into());
    }
    let mut codec = None;
    let mut idr = false;
    let mut picture = false;
    let mut pps = false;
    let mut error = None;
    {
        let mut reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
            if !nal.is_complete() {
                return NalInterest::Buffer;
            }
            let inspected = (|| -> Result<(), String> {
                let header = nal.header().map_err(|_| "invalid: H.264 NAL header")?;
                match header.nal_unit_type() {
                    UnitType::SeqParameterSet => {
                        let sps = SeqParameterSet::from_bits(nal.rbsp_bits())
                            .map_err(|_| "invalid: H.264 SPS")?;
                        let dimensions = sps
                            .pixel_dimensions()
                            .map_err(|_| "invalid: H.264 dimensions")?;
                        if dimensions != (width, height)
                            || width == 0
                            || height == 0
                            || width > MAX_WIDTH
                            || height > MAX_HEIGHT
                        {
                            return Err(
                                "invalid: H.264 dimensions do not match video configuration".into(),
                            );
                        }
                        let mut header_bytes = [0; 4];
                        nal.reader()
                            .read_exact(&mut header_bytes)
                            .map_err(|_| "invalid: incomplete H.264 codec")?;
                        // Only AVC baseline/main/high 8-bit 4:2:0 are advertised.
                        if !matches!(header_bytes[1], 66 | 77 | 100) || header_bytes[3] > 42 {
                            return Err("invalid: unsupported H.264 profile or level".into());
                        }
                        codec = Some(format!(
                            "avc1.{:02X}{:02X}{:02X}",
                            header_bytes[1], header_bytes[2], header_bytes[3]
                        ));
                    }
                    UnitType::PicParameterSet => pps = true,
                    UnitType::SliceLayerWithoutPartitioningIdr => {
                        idr = true;
                        picture = true;
                    }
                    UnitType::SliceLayerWithoutPartitioningNonIdr => picture = true,
                    UnitType::SEI | UnitType::AccessUnitDelimiter => {}
                    _ => return Err("invalid: unsupported H.264 NAL type".into()),
                }
                Ok(())
            })();
            if let Err(reason) = inspected {
                error = Some(reason);
            }
            NalInterest::Ignore
        });
        reader.push(bytes);
        reader.reset();
    }
    if let Some(error) = error {
        return Err(error);
    }
    if !picture || key != idr || (key && (codec.is_none() || !pps)) {
        return Err("invalid: H.264 keyframe or parameter sets missing".into());
    }
    Ok(codec)
}

/// Validate signed-event routing and the bounded compressed-video payload.
///
/// The caller separately verifies signatures and channel/community access.
pub fn validate_screen_event(event: &Event, now: u64) -> Result<Uuid, String> {
    if u32::from(event.kind.as_u16()) != crate::kind::KIND_HUDDLE_SCREEN {
        return Err("invalid: not a huddle screen event".into());
    }
    let channels: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
        .collect();
    if channels.len() != 1 || channels[0].as_slice().len() != 2 {
        return Err("invalid: screen events require exactly one h tag".into());
    }
    let channel = Uuid::parse_str(&channels[0].as_slice()[1])
        .map_err(|_| "invalid: screen channel must be a UUID")?;
    if channel.is_nil() {
        return Err("invalid: screen channel cannot be nil".into());
    }
    if event.created_at.as_secs().abs_diff(now) > MAX_EVENT_AGE_SECS {
        return Err("invalid: screen event is expired".into());
    }
    if event.content.len() > MAX_CONTENT_BYTES {
        return Err("invalid: screen content exceeds limit".into());
    }
    let message: ScreenMessage =
        serde_json::from_str(&event.content).map_err(|_| "invalid: malformed screen message")?;
    let share = match &message {
        ScreenMessage::Video { share, .. } | ScreenMessage::Stopped { share } => share,
    };
    if share.session_id.is_nil()
        || share.started_at == 0
        || share.started_at > now.saturating_add(MAX_EVENT_AGE_SECS).saturating_mul(1000)
        || share.label.chars().count() > 160
        || share.label.trim().is_empty()
    {
        return Err("invalid: malformed screen share identity".into());
    }
    if let ScreenMessage::Video {
        sequence,
        codec,
        width,
        height,
        frames,
        ..
    } = message
    {
        if sequence == 0
            || sequence > 9_007_199_254_740_991
            || width == 0
            || height == 0
            || width > MAX_WIDTH
            || height > MAX_HEIGHT
            || frames.is_empty()
            || frames.len() > MAX_BATCH_FRAMES
            || codec.len() != 11
            || !codec.starts_with("avc1.")
        {
            return Err("invalid: video configuration or batch size".into());
        }
        let mut total = 0;
        let mut last_timestamp = None;
        for frame in frames {
            if frame.timestamp > 9_007_199_254_740_991
                || last_timestamp.is_some_and(|last| frame.timestamp <= last)
            {
                return Err("invalid: video timestamps are not monotonic".into());
            }
            last_timestamp = Some(frame.timestamp);
            let data = STANDARD
                .decode(&frame.data)
                .map_err(|_| "invalid: H.264 base64")?;
            total += data.len();
            if total > MAX_VIDEO_BYTES {
                return Err("invalid: video batch exceeds bandwidth limit".into());
            }
            if validate_access_unit(&data, frame.key, width, height)?
                .is_some_and(|actual| !actual.eq_ignore_ascii_case(&codec))
            {
                return Err("invalid: H.264 codec does not match configuration".into());
            }
        }
    }
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use openh264::{
        encoder::{Encoder, EncoderConfig, Profile},
        formats::YUVBuffer,
        OpenH264API,
    };

    fn share() -> ScreenShare {
        ScreenShare {
            session_id: Uuid::new_v4(),
            started_at: 100_000,
            label: "Display 1".into(),
        }
    }

    fn video() -> ScreenMessage {
        let mut encoder = Encoder::with_api_config(
            OpenH264API::from_source(),
            EncoderConfig::new().profile(Profile::Baseline),
        )
        .unwrap();
        let yuv = YUVBuffer::new(64, 48);
        let first = encoder.encode(&yuv).unwrap().to_vec();
        let codec = validate_access_unit(&first, true, 64, 48).unwrap().unwrap();
        let second = encoder.encode(&yuv).unwrap().to_vec();
        ScreenMessage::Video {
            share: share(),
            sequence: 1,
            codec,
            width: 64,
            height: 48,
            frames: vec![
                VideoAccessUnit {
                    timestamp: 0,
                    key: true,
                    data: STANDARD.encode(first),
                },
                VideoAccessUnit {
                    timestamp: 33_333,
                    key: false,
                    data: STANDARD.encode(second),
                },
            ],
        }
    }

    fn event(message: &ScreenMessage, channels: &[&str]) -> Event {
        EventBuilder::new(
            Kind::Custom(crate::kind::KIND_HUDDLE_SCREEN as u16),
            serde_json::to_string(message).unwrap(),
        )
        .tags(channels.iter().map(|id| Tag::parse(["h", *id]).unwrap()))
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&Keys::generate())
        .unwrap()
    }

    #[test]
    fn accepts_actual_h264_key_and_delta_frames_without_storing_them() {
        let channel = Uuid::new_v4().to_string();
        assert!(validate_screen_event(&event(&video(), &[&channel]), 100).is_ok());
        assert!(crate::kind::is_ephemeral(crate::kind::KIND_HUDDLE_SCREEN));
    }

    #[test]
    fn screen_routing_and_freshness_guards_are_falsifiable() {
        let channel = Uuid::new_v4().to_string();
        let stop = ScreenMessage::Stopped { share: share() };
        for tags in [
            vec![],
            vec![channel.as_str(), channel.as_str()],
            vec!["bad"],
        ] {
            assert!(validate_screen_event(&event(&stop, &tags), 100).is_err());
        }
        assert!(validate_screen_event(&event(&stop, &[&channel]), 106).is_err());
    }

    #[test]
    fn rejects_still_images_and_forged_video_dimensions_and_key_flags() {
        assert!(validate_access_unit(b"\xff\xd8\xffJPEG", true, 64, 48).is_err());
        let ScreenMessage::Video { frames, .. } = video() else {
            panic!("video")
        };
        let key = STANDARD.decode(&frames[0].data).unwrap();
        assert!(validate_access_unit(&key, true, 128, 96).is_err());
        assert!(validate_access_unit(&key, false, 64, 48).is_err());
        assert!(validate_access_unit(&vec![0; MAX_VIDEO_BYTES + 1], true, 64, 48).is_err());
    }

    #[test]
    fn rejects_timestamp_reordering_and_unbounded_batches() {
        let channel = Uuid::new_v4().to_string();
        let mut message = video();
        if let ScreenMessage::Video { frames, .. } = &mut message {
            frames[1].timestamp = frames[0].timestamp;
        }
        assert!(validate_screen_event(&event(&message, &[&channel]), 100).is_err());
        let mut message = video();
        if let ScreenMessage::Video { frames, .. } = &mut message {
            *frames = vec![frames[0].clone(); MAX_BATCH_FRAMES + 1];
        }
        assert!(validate_screen_event(&event(&message, &[&channel]), 100).is_err());
    }
}
