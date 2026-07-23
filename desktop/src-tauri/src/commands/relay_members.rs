use serde::Deserialize;
use tauri::State;

use crate::{
    app_state::AppState,
    events, nostr_convert,
    relay::{
        classify_request_error, parse_json_response, query_relay, relay_api_base_url_with_override,
        relay_error_message, submit_event,
    },
};

#[derive(Deserialize)]
struct RelayInformationDocument {
    #[serde(default)]
    supported_nips: Vec<u32>,
}

#[tauri::command]
pub async fn relay_requires_membership(state: State<'_, AppState>) -> Result<bool, String> {
    let url = format!("{}/info", relay_api_base_url_with_override(&state));
    let response = state
        .http_client
        .get(url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|error| classify_request_error(&error))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let info = parse_json_response::<RelayInformationDocument>(response).await?;
    Ok(info.supported_nips.contains(&43))
}

#[tauri::command]
pub async fn list_relay_members(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    // kind:13534 is a single replaceable event on the relay carrying all members.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [13534],
            "limit": 1
        })],
    )
    .await?;

    Ok(events
        .first()
        .map(nostr_convert::relay_members_from_event)
        .unwrap_or_else(|| serde_json::json!({ "members": [] })))
}

#[tauri::command]
pub async fn get_my_relay_membership(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [13534],
            "limit": 1
        })],
    )
    .await?;

    let Some(event) = events.first() else {
        return Ok(serde_json::json!({ "member": null }));
    };

    let members_value = nostr_convert::relay_members_from_event(event);
    let me = members_value
        .get("members")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|m| m.get("pubkey").and_then(|p| p.as_str()) == Some(my_pubkey.as_str()))
                .cloned()
        });

    Ok(serde_json::json!({ "member": me }))
}

#[tauri::command]
pub async fn add_relay_member(
    target_pubkey: String,
    role: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_relay_admin_add(&target_pubkey, &role)?;
    let result = submit_event(builder, &state).await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_relay_member(
    target_pubkey: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_relay_admin_remove(&target_pubkey)?;
    let result = submit_event(builder, &state).await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn change_relay_member_role(
    target_pubkey: String,
    new_role: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_relay_admin_change_role(&target_pubkey, &new_role)?;
    let result = submit_event(builder, &state).await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
