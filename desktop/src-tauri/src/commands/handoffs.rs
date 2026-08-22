//! GUI commands for encrypted Agent handoff records.

use buzz_core_pkg::agent_handoff::{
    build_agent_handoff_event, decrypt_agent_handoff, AgentHandoffPayload, HANDOFF_VERSION,
};
use buzz_core_pkg::kind::KIND_AGENT_HANDOFF;
use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_state::AppState;
use crate::relay::{query_relay, submit_signed_event_with_keys};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAgentHandoffRequest {
    pub recipient_pubkey: String,
    pub title: String,
    pub summary: Option<String>,
    pub history: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffSummary {
    pub event_id: String,
    pub sender_pubkey: String,
    pub created_at: u64,
    pub title: String,
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffRecord {
    pub event_id: String,
    pub sender_pubkey: String,
    pub created_at: u64,
    pub title: String,
    pub summary: Option<String>,
    pub history: String,
}

#[tauri::command]
pub async fn send_agent_handoff(
    request: SendAgentHandoffRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let recipient = PublicKey::from_hex(&request.recipient_pubkey)
        .map_err(|error| format!("recipient pubkey must be 64-hex: {error}"))?;
    let keys = state.signing_keys()?;
    if recipient == keys.public_key() {
        return Err("recipient must be a different Agent".to_string());
    }
    let payload = AgentHandoffPayload {
        version: HANDOFF_VERSION,
        title: request.title,
        summary: request.summary,
        history: request.history,
    };
    let event = build_agent_handoff_event(&keys, &recipient, &payload)
        .map_err(|error| format!("invalid handoff: {error}"))?;
    let event_id = event.id.to_hex();
    submit_signed_event_with_keys(&event, &state, &keys, None)
    .await
    .map_err(|error| format!("failed to publish handoff: {error}"))?;
    Ok(event_id)
}

#[tauri::command]
pub async fn list_agent_handoffs(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentHandoffSummary>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let keys = state.signing_keys()?;
    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_HANDOFF],
        "#p": [keys.public_key().to_hex()],
        "limit": limit,
    });
    let mut result = Vec::new();
    for event in query_relay(&state, &[filter]).await? {
        if event.verify().is_err() {
            continue;
        }
        let Ok(payload) = decrypt_agent_handoff(&keys, &event) else {
            continue;
        };
        result.push(AgentHandoffSummary {
            event_id: event.id.to_hex(),
            sender_pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            title: payload.title,
            summary: payload.summary,
        });
    }
    result.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    Ok(result)
}

#[tauri::command]
pub async fn get_agent_handoff(
    event_id: String,
    state: State<'_, AppState>,
) -> Result<AgentHandoffRecord, String> {
    let keys = state.signing_keys()?;
    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_HANDOFF],
        "ids": [event_id],
        "#p": [keys.public_key().to_hex()],
        "limit": 1,
    });
    let event = query_relay(&state, &[filter])
        .await?
        .into_iter()
        .find(|event| event.verify().is_ok())
        .ok_or_else(|| "handoff not found or not addressed to this Agent".to_string())?;
    let payload = decrypt_agent_handoff(&keys, &event)
        .map_err(|error| format!("failed to decrypt handoff: {error}"))?;
    Ok(AgentHandoffRecord {
        event_id: event.id.to_hex(),
        sender_pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        title: payload.title,
        summary: payload.summary,
        history: payload.history,
    })
}
