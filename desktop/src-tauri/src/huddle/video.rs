//! Shared consent lease and signed, bounded huddle video delivery.

use std::sync::Arc;
use std::time::Duration;

use buzz_core_pkg::huddle_screen::{HuddleVideoMessage, VideoShare};
use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app_state::AppState;

/// An independently cancellable camera or screen track with a two-batch queue.
#[derive(Debug)]
pub struct VideoCapture {
    pub(super) share: VideoShare,
    pub(super) pubkey: nostr::PublicKey,
    pub(super) cancel: CancellationToken,
    pub(super) receiver: Mutex<mpsc::Receiver<HuddleVideoMessage>>,
}

impl VideoCapture {
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
) -> Result<Option<String>, String> {
    let session = Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    let (capture, channel, lifetime) = {
        let hs = state.huddle()?;
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
        (capture, channel, hs.huddle_generation)
    };
    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();
    let pubkey = keys.public_key();
    if pubkey != capture.pubkey {
        return Err("Video sharing stopped because the signer changed".into());
    }
    let message = tokio::select! {
        _ = capture.cancel.cancelled() => return Ok(None),
        message = tokio::time::timeout(Duration::from_secs(1), async {
            capture.receiver.lock().await.recv().await
        }) => match message {
            Ok(message) => message,
            Err(_) => return Ok(None),
        },
    };
    let Some(message) = message else {
        return Ok(None);
    };
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
        || !hs.is_current_huddle(&channel, lifetime)
        || current_session != Some(session)
    {
        return Ok(None);
    }
    Ok(Some(event))
}
