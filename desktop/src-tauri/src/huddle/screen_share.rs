use tauri::State;

use crate::app_state::AppState;

use super::relay_api::ScreenRelayFrame;

/// Maximum IPC screen-share frame size. The frontend sends low-rate JPEG
/// frames and drops anything above this cap before invoking, but keep the
/// Rust boundary defensive as well.
const MAX_SCREEN_FRAME_BYTES: usize = 512 * 1024;

/// Maximum JSON control payload (cursor/share-state) for screen sharing.
const MAX_SCREEN_CONTROL_BYTES: usize = 2 * 1024;

/// Receive a compressed screen-share frame from the frontend and fan it out
/// over the huddle relay. The frame is a JPEG payload; metadata is fixed by
/// the receiver event (`image/jpeg`) to keep the realtime frame small.
#[tauri::command]
pub fn push_huddle_screen_frame(
    request: tauri::ipc::Request<'_>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            if bytes.len() > MAX_SCREEN_FRAME_BYTES {
                return Err(format!(
                    "screen frame too large: {} bytes (max {})",
                    bytes.len(),
                    MAX_SCREEN_FRAME_BYTES
                ));
            }
            if bytes.is_empty() {
                return Err("screen frame is empty".to_string());
            }
            let hs = state.huddle()?;
            let Some(ref tx) = hs.screen_relay_tx else {
                return Err("huddle screen share is not connected".to_string());
            };
            let _ = tx.try_send(ScreenRelayFrame::Video(bytes.to_vec()));
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}

/// Send a small JSON control payload for screen sharing (cursor position,
/// share start/stop state) over the huddle relay.
#[tauri::command]
pub fn push_huddle_screen_control(
    control: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(&control).map_err(|e| format!("screen control JSON: {e}"))?;
    if bytes.len() > MAX_SCREEN_CONTROL_BYTES {
        return Err(format!(
            "screen control too large: {} bytes (max {})",
            bytes.len(),
            MAX_SCREEN_CONTROL_BYTES
        ));
    }
    let hs = state.huddle()?;
    let Some(ref tx) = hs.screen_relay_tx else {
        return Err("huddle screen share is not connected".to_string());
    };
    let _ = tx.try_send(ScreenRelayFrame::Control(bytes));
    Ok(())
}
