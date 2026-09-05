use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{app_state::AppState, models::ChannelDetailInfo, nostr_convert, relay::query_relay};

#[derive(Serialize)]
pub struct ChannelSessionLink {
    pub parent_channel_id: String,
    pub session_channel_id: String,
    pub creator_pubkey: String,
    pub created_at: i64,
    pub channel: Option<ChannelDetailInfo>,
}

/// List durable Session links and child metadata stored in one parent channel.
#[tauri::command]
pub async fn list_channel_sessions(
    parent_channel_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ChannelSessionLink>, String> {
    Uuid::parse_str(&parent_channel_id)
        .map_err(|_| "parent_channel_id must be a UUID".to_string())?;
    let links = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_SESSION_LINK],
            "#h": [parent_channel_id],
            "limit": 200,
        })],
    )
    .await?;
    let mut parsed = Vec::new();
    for event in links {
        let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
            continue;
        };
        let Some(session_channel_id) = content
            .get("session_channel_id")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if Uuid::parse_str(session_channel_id).is_err() {
            continue;
        }
        parsed.push((event, session_channel_id.to_string()));
    }
    if parsed.is_empty() {
        return Ok(Vec::new());
    }
    let metadata_filters = parsed
        .iter()
        .map(|(_, session_id)| {
            serde_json::json!({
                "kinds": [39000],
                "#d": [session_id],
                "limit": 1,
            })
        })
        .collect::<Vec<_>>();
    let metadata = query_relay(&state, &metadata_filters).await?;
    let metadata_by_id = metadata
        .iter()
        .filter_map(|event| {
            nostr_convert::channel_detail_from_event(event)
                .ok()
                .map(|detail| (detail.id.clone(), detail))
        })
        .collect::<std::collections::HashMap<_, _>>();
    Ok(parsed
        .into_iter()
        .filter_map(|(event, session_channel_id)| {
            let channel = metadata_by_id.get(&session_channel_id)?;
            Some(ChannelSessionLink {
                parent_channel_id: parent_channel_id.clone(),
                session_channel_id,
                creator_pubkey: event.pubkey.to_hex(),
                created_at: event.created_at.as_secs() as i64,
                channel: Some(channel.clone()),
            })
        })
        .collect())
}
