//! Opt-in browser camera frames encoded natively, without image uploads.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use buzz_core_pkg::huddle_screen::{
    HuddleVideoMessage, VideoShare, MAX_CAMERA_HEIGHT, MAX_CAMERA_WIDTH,
};
use image::RgbaImage;
use tauri::{
    ipc::{InvokeBody, Request},
    State,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::screen::encoder::ScreenVideoEncoder;
use super::video::VideoCapture;
use crate::app_state::AppState;

/// Native encoder ownership, independent of the screen-share track.
pub struct CameraCapture {
    pub(super) video: Arc<VideoCapture>,
    encoder: Arc<Mutex<ScreenVideoEncoder>>,
    sender: mpsc::Sender<HuddleVideoMessage>,
    epoch: Instant,
}

impl std::fmt::Debug for CameraCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraCapture")
            .field("video", &self.video)
            .finish_non_exhaustive()
    }
}

impl CameraCapture {
    /// Cancel pending delivery before a huddle or identity is replaced.
    pub fn cancel(&self) {
        self.video.cancel();
    }
}

/// Reserve a consent identity before the user-granted camera acquisition.
#[tauri::command]
pub async fn start_huddle_camera(
    channel_id: String,
    device_id: String,
    label: String,
    state: State<'_, AppState>,
) -> Result<VideoShare, String> {
    if device_id.len() > 512 || label.trim().is_empty() || label.chars().count() > 160 {
        return Err("Invalid camera selection".into());
    }
    let lifetime = {
        let hs = state.huddle()?;
        if !hs.is_current_huddle(&channel_id, hs.huddle_generation) {
            return Err("Join a huddle before enabling the camera".into());
        }
        if hs.camera_share.is_some() {
            return Err("Stop the current camera before selecting another".into());
        }
        hs.huddle_generation
    };
    let pubkey = state.keys.lock().map_err(|e| e.to_string())?.public_key();
    let share = VideoShare {
        session_id: Uuid::new_v4(),
        started_at: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis(),
        )
        .map_err(|e| e.to_string())?,
        label,
    };
    let encoder_share = share.clone();
    let encoder = tokio::task::spawn_blocking(move || ScreenVideoEncoder::camera(encoder_share))
        .await
        .map_err(|e| e.to_string())??;
    let (sender, receiver) = mpsc::channel(2);
    let camera = Arc::new(CameraCapture {
        video: Arc::new(VideoCapture {
            share: share.clone(),
            pubkey,
            cancel: CancellationToken::new(),
            receiver: Mutex::new(receiver),
        }),
        encoder: Arc::new(Mutex::new(encoder)),
        sender,
        epoch: Instant::now(),
    });
    {
        let mut hs = state.huddle()?;
        if !hs.is_current_huddle(&channel_id, lifetime)
            || hs.camera_share.is_some()
            || state.keys.lock().map_err(|e| e.to_string())?.public_key() != pubkey
        {
            camera.cancel();
            return Err("Camera start was superseded".into());
        }
        hs.camera_capture = Some(camera);
        hs.camera_share = Some(share.clone());
        hs.camera_device_id = device_id;
        hs.camera_error = None;
    }
    state.emit_huddle_state_changed();
    Ok(share)
}

fn header<'a>(request: &'a Request<'_>, name: &str) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| format!("Missing or invalid {name} header"))
}

fn frame_image(width: u32, height: u32, bytes: &[u8]) -> Result<RgbaImage, String> {
    if width < 16
        || height < 16
        || width > MAX_CAMERA_WIDTH
        || height > MAX_CAMERA_HEIGHT
        || bytes.len() != width as usize * height as usize * 4
    {
        return Err("Invalid or oversized camera frame".into());
    }
    RgbaImage::from_raw(width, height, bytes.to_vec()).ok_or("Invalid camera pixels".into())
}

/// Receive bounded raw RGBA over binary IPC, never a JSON number array or JPEG.
#[tauri::command]
pub async fn push_huddle_camera_frame(
    request: Request<'_>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let session =
        Uuid::parse_str(header(&request, "x-buzz-camera-session")?).map_err(|e| e.to_string())?;
    let width = header(&request, "x-buzz-camera-width")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let height = header(&request, "x-buzz-camera-height")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let camera = {
        let hs = state.huddle()?;
        let Some(camera) = hs
            .camera_capture
            .as_ref()
            .filter(|camera| camera.video.share.session_id == session)
        else {
            return Ok(false);
        };
        let Some(channel) = hs.ephemeral_channel_id.as_deref() else {
            return Ok(false);
        };
        if !hs.is_current_huddle(channel, hs.huddle_generation) {
            return Ok(false);
        }
        Arc::clone(camera)
    };
    if camera.video.cancel.is_cancelled() {
        return Ok(false);
    }
    if state.keys.lock().map_err(|e| e.to_string())?.public_key() != camera.video.pubkey {
        return Err("Camera stopped because the signer changed".into());
    }
    // One encode in flight; a second caller cannot create an unbounded work queue.
    let Ok(mut encoder) = Arc::clone(&camera.encoder).try_lock_owned() else {
        return Ok(false);
    };
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Camera frames require a raw byte body".into());
    };
    let image = frame_image(width, height, bytes)?;
    let timestamp = u64::try_from(camera.epoch.elapsed().as_micros()).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        if camera.video.cancel.is_cancelled() {
            return Ok(false);
        }
        let batches = encoder.push(image, timestamp)?;
        for batch in batches {
            if camera.video.cancel.is_cancelled() {
                return Ok(false);
            }
            match camera.sender.try_send(batch) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => encoder.discontinuity(),
                Err(mpsc::error::TrySendError::Closed(_)) => return Ok(false),
            }
        }
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Disable only this camera identity; stale stops cannot terminate a replacement.
#[tauri::command]
pub fn stop_huddle_camera(
    session_id: String,
    error: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    {
        let mut hs = state.huddle()?;
        if hs.camera_share.as_ref().map(|share| share.session_id) != Some(session) {
            return Ok(());
        }
        hs.stop_camera_capture();
        hs.camera_error = error.map(|error| error.chars().take(300).collect());
    }
    state.emit_huddle_state_changed();
    Ok(())
}

/// Drain and sign the camera queue separately from the screen-share queue.
#[tauri::command]
pub async fn read_huddle_camera_video_chunk(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    super::video::read_video_chunk(session_id, &state, buzz_core_pkg::kind::KIND_HUDDLE_CAMERA)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::build_app_state;
    use crate::huddle::{video::read_video_chunk, HuddlePhase};
    use nostr::{JsonUtil, Kind};
    use std::time::Duration;

    fn camera() -> (Arc<AppState>, Arc<CameraCapture>) {
        let state = Arc::new(build_app_state());
        let share = VideoShare {
            session_id: Uuid::new_v4(),
            started_at: nostr::Timestamp::now().as_secs() * 1000,
            label: "Camera test".into(),
        };
        let (sender, receiver) = mpsc::channel(2);
        let capture = Arc::new(CameraCapture {
            video: Arc::new(VideoCapture {
                share: share.clone(),
                pubkey: state.keys.lock().unwrap().public_key(),
                cancel: CancellationToken::new(),
                receiver: Mutex::new(receiver),
            }),
            encoder: Arc::new(Mutex::new(
                ScreenVideoEncoder::camera(share.clone()).unwrap(),
            )),
            sender,
            epoch: Instant::now(),
        });
        {
            let mut huddle = state.huddle().unwrap();
            huddle.phase = HuddlePhase::Active;
            huddle.ephemeral_channel_id = Some(Uuid::new_v4().to_string());
            huddle.huddle_generation = 1;
            huddle.camera_share = Some(share);
            huddle.camera_capture = Some(Arc::clone(&capture));
        }
        (state, capture)
    }

    #[test]
    fn raw_camera_frames_are_bounded_and_require_exact_rgba_geometry() {
        assert!(frame_image(16, 16, &[0; 16 * 16 * 4]).is_ok());
        assert!(frame_image(16, 16, &[0; 16 * 16 * 4 - 1]).is_err());
        assert!(frame_image(0, 16, &[]).is_err());
        assert!(frame_image(1920, 1080, &[]).is_err());
    }

    #[tokio::test]
    async fn a_retired_session_or_other_track_cannot_drain_the_current_camera() {
        let (state, capture) = camera();
        capture
            .sender
            .send(HuddleVideoMessage::Stopped {
                share: capture.video.share.clone(),
            })
            .await
            .unwrap();
        let kind = buzz_core_pkg::kind::KIND_HUDDLE_CAMERA;
        assert!(read_video_chunk(Uuid::new_v4().to_string(), &state, kind)
            .await
            .unwrap()
            .is_none());
        assert!(read_video_chunk(
            capture.video.share.session_id.to_string(),
            &state,
            buzz_core_pkg::kind::KIND_HUDDLE_SCREEN,
        )
        .await
        .unwrap()
        .is_none());
        let signed = read_video_chunk(capture.video.share.session_id.to_string(), &state, kind)
            .await
            .unwrap()
            .unwrap();
        let event = nostr::Event::from_json(signed).unwrap();
        assert_eq!(event.pubkey, capture.video.pubkey);
        assert_eq!(event.kind, Kind::Custom(kind as u16));
        let channel = state
            .huddle()
            .unwrap()
            .ephemeral_channel_id
            .clone()
            .unwrap();
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["h", channel.as_str()]));
        event.verify().unwrap();
    }

    #[tokio::test]
    async fn stopping_camera_capture_wakes_an_inflight_reader() {
        let (state, capture) = camera();
        let reader_state = Arc::clone(&state);
        let session = capture.video.share.session_id.to_string();
        let reader = tokio::spawn(async move {
            read_video_chunk(
                session,
                &reader_state,
                buzz_core_pkg::kind::KIND_HUDDLE_CAMERA,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while capture.video.receiver.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        state.huddle().unwrap().stop_camera_capture();
        assert!(capture.video.cancel.is_cancelled());
        assert!(tokio::time::timeout(Duration::from_millis(250), reader)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_none());
    }
}
