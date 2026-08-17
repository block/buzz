use serde::Deserialize;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::ChannelInfo,
    nostr_convert,
    relay::{parse_command_response, query_relay, submit_event},
};

#[derive(Deserialize)]
struct OpenDmAck {
    channel_id: String,
}

#[tauri::command]
pub async fn open_dm(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    // Submit a kind:41010 dm-open event; the relay replies with the channel id
    // in its OK message payload.
    let builder = events::build_dm_open(&pubkeys)?;
    let result = submit_event(builder, &state).await?;
    let ack: OpenDmAck = parse_command_response(&result.message)?;

    // Re-fetch the channel metadata so the frontend gets the same `ChannelInfo`
    // shape as `get_channel_details`.
    let metadata = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [ack.channel_id],
            "limit": 1
        })],
    )
    .await?;

    metadata
        .first()
        .map(|ev| nostr_convert::channel_info_from_event(ev, None, None))
        .transpose()?
        .ok_or_else(|| "DM channel created but metadata not yet available".to_string())
}

#[tauri::command]
pub async fn hide_dm(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let builder = events::build_dm_hide(&channel_id)?;
    submit_event(builder, &state).await?;
    Ok(())
}
