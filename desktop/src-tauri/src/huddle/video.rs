//! Shared consent lease and signed, bounded huddle video delivery.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use buzz_core_pkg::huddle_screen::{HuddleVideoMessage, VideoShare};
use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::screen::encoder::ScreenVideoEncoder;
use crate::app_state::AppState;

const VIDEO_QUEUE_BATCHES: usize = 2;
const MAX_QUEUED_FRAME_AGE_US: u64 = 500_000;

/// An independently cancellable camera or screen track with a two-batch queue.
#[derive(Debug)]
pub struct VideoCapture {
    pub(super) share: VideoShare,
    pub(super) pubkey: nostr::PublicKey,
    pub(super) cancel: CancellationToken,
    pub(super) receiver: Mutex<mpsc::Receiver<(u64, HuddleVideoMessage)>>,
    sender: mpsc::Sender<(u64, HuddleVideoMessage)>,
    epoch: Instant,
    generation: AtomicU64,
}

impl VideoCapture {
    /// Share one monotonic clock and bounded recovery queue across either producer.
    pub(super) fn new(
        share: VideoShare,
        pubkey: nostr::PublicKey,
        cancel: CancellationToken,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(VIDEO_QUEUE_BATCHES);
        Self {
            share,
            pubkey,
            cancel,
            receiver: Mutex::new(receiver),
            sender,
            epoch: Instant::now(),
            generation: AtomicU64::new(0),
        }
    }

    /// Capture timestamps and queue-age checks use the same monotonic epoch.
    pub(super) fn timestamp(&self) -> Result<u64, String> {
        u64::try_from(self.epoch.elapsed().as_micros()).map_err(|error| error.to_string())
    }

    /// Retire dependencies and request a new IDR without reacquiring the source.
    pub(super) fn refresh(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut receiver) = self.receiver.try_lock() {
            for _ in 0..VIDEO_QUEUE_BATCHES {
                if receiver.try_recv().is_err() {
                    break;
                }
            }
        }
    }

    /// Congestion must evict old pixels, not keep them while dropping newer video.
    pub(super) fn enqueue(&self, batch: HuddleVideoMessage, generation: u64) -> Result<(), String> {
        if self.cancel.is_cancelled() || generation != self.generation.load(Ordering::SeqCst) {
            return Ok(());
        }
        match self.sender.try_send((generation, batch)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.refresh();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err("Video delivery queue closed".to_owned())
            }
        }
    }

    fn prepare_encoder(&self, encoder: &mut ScreenVideoEncoder) -> u64 {
        let generation = self.generation.load(Ordering::SeqCst);
        encoder.begin_generation(generation);
        generation
    }

    /// Encode only current dependencies, retaining the normal video capture cadence.
    pub(super) fn encode(
        &self,
        encoder: &mut ScreenVideoEncoder,
        image: image::RgbaImage,
        timestamp: u64,
    ) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        let generation = self.prepare_encoder(encoder);
        for batch in encoder.push(image, timestamp)? {
            self.enqueue(batch, generation)?;
        }
        Ok(())
    }

    /// The reader also flushes partial camera batches during sparse capture.
    pub(super) fn flush_pending(&self, encoder: &mut ScreenVideoEncoder) -> Result<(), String> {
        let generation = self.prepare_encoder(encoder);
        if let Some(batch) = encoder.flush_due(self.timestamp()?) {
            self.enqueue(batch, generation)?;
        }
        Ok(())
    }

    /// Retire queued pixels and wake any pending reader.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// Read only the current share, fencing both sides of asynchronous signing.
pub(super) async fn read_video_chunk(
    session_id: String,
    state: &AppState,
    kind: u32,
    refresh_keyframe: bool,
) -> Result<Option<String>, String> {
    let session = Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    let (capture, camera, channel, lifetime) = {
        let hs = state.huddle()?;
        let camera = hs.camera_capture.as_ref().cloned();
        let capture = match kind {
            buzz_core_pkg::kind::KIND_HUDDLE_SCREEN => hs.screen_capture.as_ref().cloned(),
            buzz_core_pkg::kind::KIND_HUDDLE_CAMERA => hs
                .camera_capture
                .as_ref()
                .map(|camera| Arc::clone(&camera.video)),
            _ => return Err("Unknown huddle video track".into()),
        };
        let Some(capture) = capture.filter(|capture| capture.share.session_id == session) else {
            return Ok(None);
        };
        let channel = hs.ephemeral_channel_id.clone().ok_or("No active huddle")?;
        if !hs.is_current_huddle(&channel, hs.huddle_generation) {
            return Ok(None);
        }
        (capture, camera, channel, hs.huddle_generation)
    };
    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();
    let pubkey = keys.public_key();
    if pubkey != capture.pubkey {
        return Err("Video sharing stopped because the signer changed".into());
    }
    if capture.cancel.is_cancelled() {
        return Ok(None);
    }
    if refresh_keyframe {
        capture.refresh();
    }
    if kind == buzz_core_pkg::kind::KIND_HUDDLE_CAMERA {
        if let Some(camera) = camera {
            camera.flush_pending()?;
        }
    }
    let read_wait = if kind == buzz_core_pkg::kind::KIND_HUDDLE_CAMERA {
        Duration::from_millis(25)
    } else {
        Duration::from_secs(1)
    };
    let message = tokio::select! {
        _ = capture.cancel.cancelled() => return Ok(None),
        message = tokio::time::timeout(read_wait, async {
            capture.receiver.lock().await.recv().await
        }) => match message {
            Ok(message) => message,
            Err(_) => return Ok(None),
        },
    };
    let Some((generation, message)) = message else {
        return Ok(None);
    };
    if generation != capture.generation.load(Ordering::SeqCst) {
        return Ok(None);
    }
    if let HuddleVideoMessage::Video { frames, .. } = &message {
        let now = capture.timestamp()?;
        if frames
            .first()
            .is_some_and(|frame| now.saturating_sub(frame.timestamp) > MAX_QUEUED_FRAME_AGE_US)
        {
            capture.refresh();
            return Ok(None);
        }
    }
    let event_channel = channel.clone();
    let event = tokio::task::spawn_blocking(move || {
        EventBuilder::new(
            Kind::Custom(kind as u16),
            serde_json::to_string(&message).map_err(|e| e.to_string())?,
        )
        .tags([Tag::parse(["h", event_channel.as_str()]).map_err(|e| e.to_string())?])
        .sign_with_keys(&keys)
        .map(|event| event.as_json())
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    if state.keys.lock().map_err(|e| e.to_string())?.public_key() != pubkey {
        return Err("Video sharing stopped because the signer changed".into());
    }
    let hs = state.huddle()?;
    let current_session = match kind {
        buzz_core_pkg::kind::KIND_HUDDLE_SCREEN => hs.screen_share.as_ref(),
        buzz_core_pkg::kind::KIND_HUDDLE_CAMERA => hs.camera_share.as_ref(),
        _ => None,
    }
    .map(|share| share.session_id);
    if capture.cancel.is_cancelled()
        || generation != capture.generation.load(Ordering::SeqCst)
        || !hs.is_current_huddle(&channel, lifetime)
        || current_session != Some(session)
    {
        return Ok(None);
    }
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use buzz_core_pkg::huddle_screen::VideoAccessUnit;

    fn capture() -> VideoCapture {
        VideoCapture::new(
            VideoShare {
                session_id: Uuid::new_v4(),
                started_at: nostr::Timestamp::now().as_secs() * 1000,
                label: "Queue latency regression".into(),
            },
            nostr::Keys::generate().public_key(),
            CancellationToken::new(),
        )
    }

    fn packet(capture: &VideoCapture, sequence: u64) -> HuddleVideoMessage {
        HuddleVideoMessage::Video {
            share: capture.share.clone(),
            sequence,
            codec: "avc1.42c01f".into(),
            width: 320,
            height: 180,
            frames: vec![VideoAccessUnit {
                timestamp: 0,
                key: sequence == 1,
                data: "AAAAAWUAAAA=".into(),
            }],
        }
    }

    #[test]
    fn queue_congestion_retires_old_dependencies_instead_of_preserving_old_pixels() {
        let capture = capture();
        capture.enqueue(packet(&capture, 1), 0).unwrap();
        capture.enqueue(packet(&capture, 2), 0).unwrap();
        capture.enqueue(packet(&capture, 3), 0).unwrap();
        assert!(capture.receiver.try_lock().unwrap().try_recv().is_err());
        assert_eq!(capture.generation.load(Ordering::SeqCst), 1);
        capture.enqueue(packet(&capture, 4), 0).unwrap();
        assert!(capture.receiver.try_lock().unwrap().try_recv().is_err());
    }

    #[test]
    fn the_next_encoded_frame_after_congestion_is_a_fresh_decodable_keyframe() {
        let capture = capture();
        let mut encoder = ScreenVideoEncoder::camera(capture.share.clone()).unwrap();
        capture
            .encode(&mut encoder, image::RgbaImage::new(320, 180), 0)
            .unwrap();
        capture.enqueue(packet(&capture, 2), 0).unwrap();
        capture.enqueue(packet(&capture, 3), 0).unwrap();
        capture
            .encode(&mut encoder, image::RgbaImage::new(320, 180), 100_000)
            .unwrap();
        let (generation, batch) = capture.receiver.try_lock().unwrap().try_recv().unwrap();
        assert_eq!(generation, 1);
        let HuddleVideoMessage::Video { frames, .. } = batch else {
            panic!("expected video");
        };
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].timestamp, 100_000);
        assert!(frames[0].key);
        let data = base64::engine::general_purpose::STANDARD
            .decode(&frames[0].data)
            .unwrap();
        assert!(openh264::decoder::Decoder::new()
            .unwrap()
            .decode(&data)
            .unwrap()
            .is_some());
    }

    #[test]
    fn signing_completion_cannot_revive_a_retired_queue_generation() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let state = crate::app_state::build_app_state();
            let mut capture = capture();
            capture.pubkey = state.keys.lock().unwrap().public_key();
            let capture = Arc::new(capture);
            {
                let mut huddle = state.huddle().unwrap();
                huddle.phase = crate::huddle::HuddlePhase::Active;
                huddle.ephemeral_channel_id = Some(Uuid::new_v4().to_string());
                huddle.huddle_generation = 1;
                huddle.screen_share = Some(capture.share.clone());
                huddle.screen_capture = Some(Arc::clone(&capture));
            }
            capture.enqueue(packet(&capture, 1), 0).unwrap();
            let (release, held) = std::sync::mpsc::sync_channel(1);
            let (ready, started) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                ready.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(2)).unwrap();
            });
            started.await.unwrap();
            let read = read_video_chunk(
                capture.share.session_id.to_string(),
                &state,
                buzz_core_pkg::kind::KIND_HUDDLE_SCREEN,
                false,
            );
            tokio::pin!(read);
            // The sole blocking worker is occupied, so this poll reaches signing
            // but cannot complete it. No sleep or test-only signing replacement.
            assert!(futures_util::poll!(read.as_mut()).is_pending());
            assert!(capture.receiver.try_lock().unwrap().is_empty());
            capture.refresh();
            release.send(()).unwrap();
            blocker.await.unwrap();
            assert!(read.await.unwrap().is_none());
        });
    }
}
