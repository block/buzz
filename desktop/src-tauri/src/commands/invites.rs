use std::time::Duration;

use serde_json::{json, Value};
use tauri::State;

use crate::{app_state::AppState, relay::post_relay_json};

// Surface an unreachable relay in the invite dialog within seconds instead of
// waiting for the OS connect timeout.
const MINT_INVITE_TIMEOUT: Duration = Duration::from_secs(15);

/// Mint an invite on the active community's relay (owner/admin only).
///
/// Sent natively rather than from the WebView: macOS App Transport Security
/// blocks WebView requests to plain-HTTP relays before they leave the machine.
#[tauri::command]
pub async fn mint_invite(
    ttl_secs: Option<u64>,
    max_uses: Option<i32>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    post_relay_json(
        &state,
        "/api/invites",
        &json!({ "ttl_secs": ttl_secs, "max_uses": max_uses }),
        MINT_INVITE_TIMEOUT,
    )
    .await
}
